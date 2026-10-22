//! hidraw access without libudev: enumerate via sysfs, talk via /dev/hidrawN.
use anyhow::{Context, Result, bail};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct HidInfo {
    pub node: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: String,
    pub interface: i32,
    pub usage_page: u32,
    pub usage: u32,
}

pub fn enumerate() -> Vec<HidInfo> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir("/sys/class/hidraw") else { return out };
    for e in rd.flatten() {
        let node = e.file_name().to_string_lossy().into_owned();
        let dev = e.path().join("device");
        let Ok(uevent) = fs::read_to_string(dev.join("uevent")) else { continue };
        let (mut vid, mut pid, mut name) = (0u16, 0u16, String::new());
        for l in uevent.lines() {
            if let Some(v) = l.strip_prefix("HID_ID=") {
                let p: Vec<&str> = v.split(':').collect();
                if p.len() == 3 {
                    vid = u32::from_str_radix(p[1], 16).unwrap_or(0) as u16;
                    pid = u32::from_str_radix(p[2], 16).unwrap_or(0) as u16;
                }
            } else if let Some(v) = l.strip_prefix("HID_NAME=") {
                name = v.to_string();
            }
        }
        // The USB interface directory is the HID device's parent.
        let interface = fs::canonicalize(&dev)
            .ok()
            .and_then(|p| fs::read_to_string(p.parent()?.join("bInterfaceNumber")).ok())
            .and_then(|s| i32::from_str_radix(s.trim(), 16).ok())
            .unwrap_or(-1);
        let (usage_page, usage) = report_descriptor_usage(&e.path().join("device/report_descriptor"));
        out.push(HidInfo { node, vendor_id: vid, product_id: pid, name, interface, usage_page, usage });
    }
    out.sort_by(|a, b| a.node.cmp(&b.node));
    out
}

/// First top-level Usage Page / Usage items from the HID report descriptor.
fn report_descriptor_usage(p: &PathBuf) -> (u32, u32) {
    let Ok(d) = fs::read(p) else { return (0, 0) };
    let (mut page, mut usage, mut i) = (0u32, 0u32, 0usize);
    while i < d.len() && (page == 0 || usage == 0) {
        let b = d[i];
        let size = match b & 3 { 3 => 4, n => n as usize };
        let val = d[i + 1..(i + 1 + size).min(d.len())].iter().rev().fold(0u32, |a, &x| (a << 8) | x as u32);
        match b & 0xFC {
            0x04 if page == 0 => page = val,
            0x08 if usage == 0 => usage = val,
            _ => {}
        }
        i += 1 + size;
    }
    (page, usage)
}

pub struct HidDevice {
    file: File,
}

// _IOC(dir=read|write, 'H', nr, len)
fn ioc(nr: u32, len: usize) -> libc::c_ulong {
    ((3u32 << 30) | ((len as u32) << 16) | ((b'H' as u32) << 8) | nr) as libc::c_ulong
}

impl HidDevice {
    pub fn open(node: &str) -> Result<Self> {
        let path = format!("/dev/{node}");
        let file = OpenOptions::new().read(true).write(true).open(&path)
            .with_context(|| format!("opening {path} (is a udev rule granting access installed?)"))?;
        Ok(Self { file })
    }
    pub fn write(&mut self, data: &[u8]) -> Result<usize> { Ok(self.file.write(data)?) }
    pub fn read(&mut self, len: usize, timeout_ms: i32) -> Result<Vec<u8>> {
        let mut p = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut p, 1, timeout_ms) };
        if r <= 0 { return Ok(Vec::new()); }
        let mut buf = vec![0u8; len];
        let n = self.file.read(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }
    pub fn send_feature(&mut self, data: &[u8]) -> Result<usize> {
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), ioc(0x06, data.len()), data.as_ptr()) };
        if r < 0 { bail!("HIDIOCSFEATURE: {}", std::io::Error::last_os_error()); }
        Ok(r as usize)
    }
    pub fn get_feature(&mut self, data: &mut [u8]) -> Result<usize> {
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), ioc(0x07, data.len()), data.as_mut_ptr()) };
        if r < 0 { bail!("HIDIOCGFEATURE: {}", std::io::Error::last_os_error()); }
        Ok(r as usize)
    }
}
