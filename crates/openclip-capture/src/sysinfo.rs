//! What kind of machine this is, for anonymous problem reports: the Windows build and the graphics
//! adapters with their driver versions. Nothing here identifies a person or a computer.

use serde::Serialize;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1,
};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{Interface, PCWSTR, w};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuAdapter {
    pub name: String,
    pub vendor: String,
    pub dedicated_memory_mb: u64,
    /// The user-mode driver version, such as `31.0.101.4502`, when Windows reports one.
    pub driver_version: Option<String>,
    /// Windows' own fallback adapter, not a real graphics card.
    pub software: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub windows: String,
    pub adapters: Vec<GpuAdapter>,
}

/// Reads the Windows build and graphics adapters. Failures leave the matching part empty.
#[must_use]
pub fn describe_system() -> SystemInfo {
    SystemInfo {
        windows: windows_version(),
        adapters: adapters().unwrap_or_default(),
    }
}

fn vendor_name(vendor_id: u32) -> &'static str {
    match vendor_id {
        0x10DE => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x8086 => "Intel",
        0x1414 => "Microsoft",
        _ => "Other",
    }
}

fn adapters() -> windows::core::Result<Vec<GpuAdapter>> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }?;
    let mut found = Vec::new();
    let mut index = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
        index += 1;
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let name_end = desc
            .Description
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(desc.Description.len());
        found.push(GpuAdapter {
            name: String::from_utf16_lossy(&desc.Description[..name_end]),
            vendor: vendor_name(desc.VendorId).to_owned(),
            dedicated_memory_mb: u64::try_from(desc.DedicatedVideoMemory).unwrap_or(0) / (1 << 20),
            driver_version: driver_version(&adapter),
            software: desc.Flags & u32::try_from(DXGI_ADAPTER_FLAG_SOFTWARE.0).unwrap_or(0) != 0,
        });
    }
    Ok(found)
}

fn driver_version(adapter: &IDXGIAdapter1) -> Option<String> {
    let version = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }.ok()?;
    let value = u64::try_from(version).ok()?;
    Some(format!(
        "{}.{}.{}.{}",
        value >> 48,
        (value >> 32) & 0xffff,
        (value >> 16) & 0xffff,
        value & 0xffff
    ))
}

fn registry_text(name: PCWSTR) -> Option<String> {
    let mut buffer = [0_u16; 128];
    let mut bytes = u32::try_from(std::mem::size_of_val(&buffer)).ok()?;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut bytes),
        )
    };
    if status.is_err() {
        return None;
    }
    let units = (bytes as usize / 2).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..units]);
    let text = text.trim_end_matches('\0').trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn windows_version() -> String {
    let build = registry_text(w!("CurrentBuildNumber"));
    let display = registry_text(w!("DisplayVersion"));
    // Windows 11 still reports "Windows 10" as its product name; the build number tells them apart.
    let family = match build.as_deref().and_then(|build| build.parse::<u32>().ok()) {
        Some(number) if number >= 22_000 => "Windows 11",
        Some(_) => "Windows 10",
        None => "Windows",
    };
    [
        Some(family.to_owned()),
        display,
        build.map(|build| format!("build {build}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_this_machine_without_identifying_it() {
        let info = describe_system();
        eprintln!("{info:?}");
        assert!(info.windows.starts_with("Windows"), "{}", info.windows);
        assert!(
            !info.adapters.is_empty(),
            "a machine has at least one adapter"
        );
        let json = format!("{info:?}");
        let user = std::env::var("USERNAME").unwrap_or_default();
        if user.len() >= 3 {
            assert!(!json.to_lowercase().contains(&user.to_lowercase()));
        }
    }
}
