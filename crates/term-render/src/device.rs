//! D3D11 device creation with WARP fallback.
//!
//! Per SPEC §6.2 (D3) and UC-04 step 1: create a BGRA-capable D3D11 device at
//! feature level 11_0 or higher. `DriverType` is an explicit parameter so tests
//! can force the WARP software rasterizer (headless CI has no real GPU we can
//! rely on).

use windows::core::Result;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL,
    D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_SDK_VERSION,
};

/// Which rasterizer to create the device on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverPreference {
    /// Real GPU; falls back to WARP automatically if hardware creation fails.
    HardwareThenWarp,
    /// Force the WARP software rasterizer (used by CI smoke tests).
    Warp,
}

/// A created D3D11 device plus its immediate context and the driver type that
/// actually succeeded.
pub struct Device {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub driver_type: D3D_DRIVER_TYPE,
    pub feature_level: D3D_FEATURE_LEVEL,
}

impl Device {
    /// Create a BGRA-capable feature-level 11_0+ device.
    ///
    /// `BGRA_SUPPORT` is required for composition/DirectComposition interop and
    /// for DirectWrite/D2D interop later in the pipeline.
    pub fn create(pref: DriverPreference) -> Result<Self> {
        match pref {
            DriverPreference::Warp => Self::create_with_driver(D3D_DRIVER_TYPE_WARP),
            DriverPreference::HardwareThenWarp => {
                match Self::create_with_driver(D3D_DRIVER_TYPE_HARDWARE) {
                    Ok(dev) => Ok(dev),
                    Err(_) => Self::create_with_driver(D3D_DRIVER_TYPE_WARP),
                }
            }
        }
    }

    fn create_with_driver(driver_type: D3D_DRIVER_TYPE) -> Result<Self> {
        // Request 11_1 then 11_0; the runtime picks the highest available.
        let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];

        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut obtained_level = D3D_FEATURE_LEVEL_11_0;

        // SAFETY: all out-params are valid pointers to properly-typed locals;
        // the feature-level slice outlives the call.
        unsafe {
            D3D11CreateDevice(
                None,
                driver_type,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut obtained_level),
                Some(&mut context),
            )?;
        }

        // On success D3D11CreateDevice guarantees both out-pointers are set.
        let device = device.expect("D3D11CreateDevice succeeded but device is null");
        let context = context.expect("D3D11CreateDevice succeeded but context is null");

        Ok(Self {
            device,
            context,
            driver_type,
            feature_level: obtained_level,
        })
    }

    /// True if this device runs on the WARP software rasterizer.
    pub fn is_warp(&self) -> bool {
        self.driver_type == D3D_DRIVER_TYPE_WARP
    }
}
