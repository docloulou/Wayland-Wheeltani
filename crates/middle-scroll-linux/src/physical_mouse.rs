use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

use evdev::Device;

use crate::errors::DaemonError;

#[derive(Debug)]
pub struct PhysicalMouse {
    path: PathBuf,
    device: Device,
    grabbed: bool,
}

impl PhysicalMouse {
    pub fn open(path: &Path) -> Result<Self, DaemonError> {
        if !path.exists() {
            return Err(DaemonError::DeviceNotFound(path.to_path_buf()));
        }

        let device = Device::open(path).map_err(|source| DaemonError::OpenDevice {
            path: path.to_path_buf(),
            source,
        })?;

        if !looks_like_mouse(&device) {
            return Err(DaemonError::NotAMouse {
                path: path.to_path_buf(),
            });
        }

        Ok(Self {
            path: path.to_path_buf(),
            device,
            grabbed: false,
        })
    }

    pub fn grab(&mut self) -> Result<(), DaemonError> {
        if self.grabbed {
            return Ok(());
        }
        self.device
            .grab()
            .map_err(|source| DaemonError::GrabFailed {
                path: self.path.clone(),
                source,
            })?;
        self.grabbed = true;
        Ok(())
    }

    pub fn ungrab(&mut self) {
        if !self.grabbed {
            return;
        }
        if let Err(err) = self.device.ungrab() {
            tracing::warn!(?err, path=%self.path.display(), "ungrab failed");
        }
        self.grabbed = false;
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.device.as_fd()
    }

    pub fn fetch_events(&mut self) -> std::io::Result<evdev::FetchEventsSynced<'_>> {
        self.device.fetch_events()
    }

    pub fn name(&self) -> &str {
        self.device.name().unwrap_or("(unknown)")
    }
}

impl Drop for PhysicalMouse {
    fn drop(&mut self) {
        self.ungrab();
    }
}

fn looks_like_mouse(dev: &Device) -> bool {
    let has_btn_left = dev
        .supported_keys()
        .is_some_and(|k| k.contains(evdev::KeyCode::BTN_LEFT));
    let has_rel_xy = dev.supported_relative_axes().is_some_and(|a| {
        a.contains(evdev::RelativeAxisCode::REL_X) && a.contains(evdev::RelativeAxisCode::REL_Y)
    });
    has_btn_left && has_rel_xy
}
