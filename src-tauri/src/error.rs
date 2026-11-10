use idevice::IdeviceError;
use isideload::SideloadError;
use rootcause::Report;
use serde::Serialize;
use serde::ser::{SerializeStruct, Serializer};

#[derive(Debug, thiserror::Error, Clone, strum::AsRefStr)]
#[strum(serialize_all = "snake_case")]
pub enum AppError {
    #[error("{0}")]
    MaxApps(String),
    #[error("{0}")]
    NotEnoughAppIds(String),
    #[error("{0}")]
    DeviceComs(String),
    #[error("{0}")]
    InstallProhibited(String),
    #[error("{0}")]
    Underage(String),
    #[error("{0}")]
    AccountLocked(String),
    #[error("{0}")]
    Developer(String),
    #[error("{0}")]
    Auth(String),
    #[error("{0}")]
    Download(String),
    #[error("{0}: {1}")]
    HouseArrest(String, String),
    #[error("{0}")]
    RemotePairing(String),
    #[error("{0}: {1}")]
    LockdownPairing(String, String),
    #[error("{0} canceled")]
    Canceled(String),
    #[error("Failed to emit status to frontend: {0}")]
    OperationUpdate(String),
    #[error("{0}: {1}")]
    DeviceComsWithMessage(String, String),
    #[error("{0}: {1}")]
    Usbmuxd(String, String),
    #[error("Not logged in")]
    NotLoggedIn,
    #[error("No device selected")]
    NoDeviceSelected,
    #[error("{0}")]
    Anisette(String),
    #[error("{0}")]
    Keyring(String),
    #[error("Keyring error: {0} - {1}")]
    KeyringWithMessage(String, String),
    #[error("{0}: {1}")]
    Storage(String, String),
    #[error("{0}")]
    Misc(String),
    #[error("{0}: {1}")]
    Filesystem(String, String),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("type", self.as_ref())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

const INSTALL_PROHIBITED_MESSAGE: &str = "The iPhone refused app installation (InstallProhibited). Check whether Screen Time or a device management policy prohibits app installation. If this iPhone is managed, contact its administrator.";

fn is_install_prohibited(error: &IdeviceError) -> bool {
    // idevice preserves an unrecognized device code, optionally followed by the
    // device's ErrorDescription in parentheses. Match that code, not arbitrary
    // report text that might only mention it while describing a different error.
    matches!(error, IdeviceError::UnknownErrorType(code)
        if code == "InstallProhibited"
            || (code.starts_with("InstallProhibited (") && code.ends_with(')')))
}

// from rootcause report
impl From<Report> for AppError {
    fn from(report: Report) -> Self {
        let report_str = report.to_string();

        for cause in report.iter_reports() {
            if cause.downcast_current_context::<IdeviceError>().is_some_and(is_install_prohibited) {
                return AppError::InstallProhibited(INSTALL_PROHIBITED_MESSAGE.into());
            }
            if cause.downcast_current_context::<keyring::Error>().is_some() {
                return AppError::Keyring(report_str);
            }
            if let Some(err) = cause.downcast_current_context::<SideloadError>() {
                match err {
                    &SideloadError::AuthWithMessage(code, _) => match code {
                        -20209 => return AppError::AccountLocked(report_str),
                        _ => {
                            return AppError::Auth(report_str);
                        }
                    },
                    &SideloadError::DeveloperError(code, _) => match code {
                        1102 => return AppError::Underage(report_str),
                        _ => {
                            return AppError::Developer(report_str);
                        }
                    },
                    SideloadError::IdeviceError(idev_err) => match idev_err {
                        error if is_install_prohibited(error) => {
                            return AppError::InstallProhibited(INSTALL_PROHIBITED_MESSAGE.into());
                        }
                        IdeviceError::Socket(_) => {
                            return AppError::DeviceComs(report_str);
                        }
                        IdeviceError::ApplicationVerificationFailed(e) => {
                            if e.contains("maximum number of installed apps") {
                                return AppError::MaxApps(report_str);
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            let cause_str = cause.to_string();
            if cause_str.contains("Not enough available app IDs") {
                return AppError::NotEnoughAppIds(report_str);
            }
            if cause_str.contains("Failed to get anisette data for login")
                || cause_str.contains("Failed to get anisette client info")
                || cause_str.contains("Failed to get anisette headers")
            {
                return AppError::Anisette(report_str);
            }
        }

        AppError::Misc(report_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_device_install_prohibition_has_clear_help_without_internal_trace() {
        for code in ["InstallProhibited", "InstallProhibited (App installation is restricted)"] {
            let report = Report::new(SideloadError::IdeviceError(IdeviceError::UnknownErrorType(code.into())))
                .context("Failed at C:\\internal\\sideloader.rs:316")
                .context("Installing signed app")
                .into_dynamic();
            let error = AppError::from(report);
            assert!(matches!(error, AppError::InstallProhibited(_)));
            assert_eq!(error.to_string(), INSTALL_PROHIBITED_MESSAGE);
            let serialized = serde_json::to_value(&error).unwrap();
            assert_eq!(serialized["type"], "install_prohibited");
            assert!(!serialized["message"].as_str().unwrap().contains("sideloader.rs"));
        }
        let direct = Report::new(IdeviceError::UnknownErrorType("InstallProhibited".into())).into_dynamic();
        assert!(matches!(AppError::from(direct), AppError::InstallProhibited(_)));
    }

    #[test]
    fn unrelated_errors_and_mentions_are_not_install_prohibitions() {
        for code in ["ApplicationVerificationFailed", "InstallProhibitedElsewhere", "OtherError (InstallProhibited)"] {
            let report = Report::new(SideloadError::IdeviceError(IdeviceError::UnknownErrorType(code.into())))
                .context("Installing signed app").into_dynamic();
            assert!(matches!(AppError::from(report), AppError::Misc(_)));
        }
        let mention = Report::new(std::io::Error::other("See InstallProhibited documentation for a different error")).into_dynamic();
        assert!(matches!(AppError::from(mention), AppError::Misc(_)));
        let authentication = Report::new(SideloadError::AuthWithMessage(-1, "InstallProhibited".into())).into_dynamic();
        assert!(matches!(AppError::from(authentication), AppError::Auth(_)));
    }
}
