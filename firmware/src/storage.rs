use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_sys::{
    esp, nvs_close, nvs_commit, nvs_erase_all, nvs_handle_t, nvs_open,
    nvs_open_mode_t_NVS_READWRITE, EspError,
};
use fridge_guard_core::activity::ActivityState;
use fridge_guard_core::config::DeviceConfig;
use fridge_guard_core::quota::QuotaState;
use fridge_guard_core::scheduler::SchedulerState;
use log::{error, info, warn};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::ffi::CStr;

pub const NVS_NAMESPACE: &str = "fridge_guard";
const NVS_NAMESPACE_C: &CStr = c"fridge_guard";
/// Namespace where the ESP-IDF Wi-Fi driver keeps its own copy of the station credentials.
/// Current firmware runs Wi-Fi without NVS storage, but older builds may have written it.
const WIFI_DRIVER_NVS_NAMESPACE_C: &CStr = c"nvs.net80211";
const KEY_CONFIG: &str = "config";
const KEY_SCHEDULER: &str = "scheduler";
const KEY_QUOTA: &str = "quota";
const KEY_ACTIVITY: &str = "activity";

pub struct StorageManager {
    nvs: EspNvs<NvsDefault>,
    /// Set by a factory reset: nothing may be written again before the device restarts
    sealed: bool,
}

impl StorageManager {
    pub fn new(nvs_default_partition: EspDefaultNvsPartition) -> Result<Self, EspError> {
        let nvs = EspNvs::new(nvs_default_partition, NVS_NAMESPACE, true)?;
        Ok(Self { nvs, sealed: false })
    }

    /// Load device configuration from NVS. Returns Default if not yet provisioned.
    pub fn load_config(&self) -> DeviceConfig {
        let mut buf = [0u8; 2048];
        match self.nvs.get_str(KEY_CONFIG, &mut buf) {
            Ok(Some(json_str)) => match serde_json::from_str::<DeviceConfig>(json_str) {
                Ok(cfg) => {
                    info!("Loaded configuration from NVS");
                    return cfg;
                }
                Err(e) => {
                    error!("Failed to parse config from NVS: {e}");
                }
            },
            Ok(None) => {
                info!("No configuration found in NVS; using unprovisioned defaults");
            }
            Err(e) => {
                error!("Error reading config from NVS: {e:?}");
            }
        }
        DeviceConfig::default()
    }

    /// Save device configuration to NVS
    pub fn save_config(&mut self, config: &DeviceConfig) -> Result<(), EspError> {
        self.save_json(KEY_CONFIG, config)?;
        info!("Saved configuration to NVS");
        Ok(())
    }

    /// Load scheduler state (last sent date, pending report)
    pub fn load_scheduler_state(&self) -> Option<SchedulerState> {
        self.load_json(KEY_SCHEDULER, &mut [0u8; 1024])
    }

    /// Save scheduler state (last sent date, pending report)
    pub fn save_scheduler_state(&mut self, state: &SchedulerState) -> Result<(), EspError> {
        self.save_json(KEY_SCHEDULER, state)
    }

    /// Load monthly quota state
    pub fn load_quota_state(&self) -> Option<QuotaState> {
        self.load_json(KEY_QUOTA, &mut [0u8; 256])
    }

    /// Save monthly quota state
    pub fn save_quota_state(&mut self, state: &QuotaState) -> Result<(), EspError> {
        self.save_json(KEY_QUOTA, state)
    }

    /// Load activity state (today's count, last use, inactivity alert latch)
    pub fn load_activity_state(&self) -> Option<ActivityState> {
        self.load_json(KEY_ACTIVITY, &mut [0u8; 512])
    }

    /// Save activity state (today's count, last use, inactivity alert latch)
    pub fn save_activity_state(&mut self, state: &ActivityState) -> Result<(), EspError> {
        self.save_json(KEY_ACTIVITY, state)
    }

    /// Factory reset: erase this firmware's namespace and the Wi-Fi driver's credential copy.
    /// Afterwards the storage refuses further writes, so state saved by other tasks cannot
    /// reappear before the restart that must follow.
    pub fn factory_reset(&mut self) -> Result<(), EspError> {
        info!("Executing factory reset: erasing NVS namespaces '{NVS_NAMESPACE}' and Wi-Fi credentials");
        self.sealed = true;
        erase_namespace(NVS_NAMESPACE_C)?;
        erase_namespace(WIFI_DRIVER_NVS_NAMESPACE_C)?;
        Ok(())
    }

    fn load_json<T: DeserializeOwned>(&self, key: &str, buf: &mut [u8]) -> Option<T> {
        match self.nvs.get_str(key, buf) {
            Ok(Some(json_str)) => match serde_json::from_str::<T>(json_str) {
                Ok(state) => Some(state),
                Err(e) => {
                    warn!("Ignoring unreadable '{key}' state in NVS: {e}");
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                warn!("Error reading '{key}' from NVS: {e:?}");
                None
            }
        }
    }

    fn save_json<T: Serialize>(&mut self, key: &str, value: &T) -> Result<(), EspError> {
        if self.sealed {
            info!("Not saving '{key}': factory reset in progress");
            return Ok(());
        }
        let json_str = serde_json::to_string(value).map_err(|e| {
            error!("Failed to serialize '{key}': {e}");
            EspError::from_infallible::<
                { esp_idf_sys::ESP_ERR_INVALID_ARG as esp_idf_sys::esp_err_t },
            >()
        })?;
        self.nvs.set_str(key, &json_str)
    }
}

fn erase_namespace(namespace: &CStr) -> Result<(), EspError> {
    let mut handle: nvs_handle_t = 0;
    esp!(unsafe {
        nvs_open(
            namespace.as_ptr(),
            nvs_open_mode_t_NVS_READWRITE,
            &mut handle,
        )
    })?;
    let result =
        esp!(unsafe { nvs_erase_all(handle) }).and_then(|_| esp!(unsafe { nvs_commit(handle) }));
    unsafe { nvs_close(handle) };
    result
}
