//! SMBus access via /dev/i2c-N (i2c-dev). Read-only unless a write is explicitly allowed.
use anyhow::{Context, Result, bail};
use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;

const I2C_SLAVE: libc::c_ulong = 0x0703;
const I2C_SMBUS: libc::c_ulong = 0x0720;
const SMBUS_READ: u8 = 1;
const SMBUS_BYTE: u32 = 1;
const SMBUS_BYTE_DATA: u32 = 2;
const SMBUS_WORD_DATA: u32 = 3;
const SMBUS_BLOCK_DATA: u32 = 5;

#[repr(C)]
union SmbusData { byte: u8, word: u16, block: [u8; 34] }

#[repr(C)]
struct SmbusIoctl { read_write: u8, command: u8, size: u32, data: *mut SmbusData }

/// SPD hubs and EEPROMs live here; writing them can brick a DIMM's profile.
fn is_protected(addr: u16) -> bool { (0x50..=0x57).contains(&addr) }

pub struct Bus { file: File, pub num: u32, addr: Option<u16>, pub writable: bool }

pub struct Adapter { pub num: u32, pub name: String }

pub fn adapters() -> Vec<Adapter> {
    let mut v: Vec<Adapter> = fs::read_dir("/sys/bus/i2c/devices").into_iter().flatten().flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().strip_prefix("i2c-")?.parse().ok()?;
            let name = fs::read_to_string(e.path().join("name")).ok()?.trim().to_string();
            Some(Adapter { num: n, name })
        }).collect();
    v.sort_by_key(|a| a.num);
    v
}

impl Bus {
    pub fn open(num: u32, writable: bool) -> Result<Self> {
        let path = format!("/dev/i2c-{num}");
        let file = OpenOptions::new().read(true).write(true).open(&path).with_context(|| format!("open {path}"))?;
        Ok(Self { file, num, addr: None, writable })
    }

    /// Select a target. Fails with EBUSY if a kernel driver already owns the address.
    pub fn set_addr(&mut self, addr: u16) -> Result<()> {
        if unsafe { libc::ioctl(self.file.as_raw_fd(), I2C_SLAVE, addr as libc::c_ulong) } < 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| format!("select {addr:#04x}"));
        }
        self.addr = Some(addr);
        Ok(())
    }

    fn xfer(&self, rw: u8, cmd: u8, size: u32, data: &mut SmbusData) -> std::io::Result<()> {
        let mut a = SmbusIoctl { read_write: rw, command: cmd, size, data };
        if unsafe { libc::ioctl(self.file.as_raw_fd(), I2C_SMBUS, &mut a as *mut _) } < 0 {
            Err(std::io::Error::last_os_error())
        } else { Ok(()) }
    }

    /// Receive-byte: no register, no data written. Used for passive presence detection.
    pub fn read_byte(&self) -> std::io::Result<u8> {
        let mut d = SmbusData { word: 0 };
        self.xfer(SMBUS_READ, 0, SMBUS_BYTE, &mut d)?;
        Ok(unsafe { d.byte })
    }

    pub fn read_byte_data(&self, reg: u8) -> std::io::Result<u8> {
        let mut d = SmbusData { word: 0 };
        self.xfer(SMBUS_READ, reg, SMBUS_BYTE_DATA, &mut d)?;
        Ok(unsafe { d.byte })
    }

    pub fn write_byte_data(&self, reg: u8, v: u8) -> Result<()> {
        self.check_write()?;
        let mut d = SmbusData { byte: v };
        self.xfer(0, reg, SMBUS_BYTE_DATA, &mut d).map_err(Into::into)
    }

    pub fn write_word_data(&self, reg: u8, v: u16) -> Result<()> {
        self.check_write()?;
        let mut d = SmbusData { word: v };
        self.xfer(0, reg, SMBUS_WORD_DATA, &mut d).map_err(Into::into)
    }

    pub fn write_block_data(&self, reg: u8, data: &[u8]) -> Result<()> {
        self.check_write()?;
        if data.len() > 32 { bail!("block too long"); }
        let mut b = [0u8; 34];
        b[0] = data.len() as u8;
        b[1..=data.len()].copy_from_slice(data);
        let mut d = SmbusData { block: b };
        self.xfer(0, reg, SMBUS_BLOCK_DATA, &mut d).map_err(Into::into)
    }

    pub fn ene_write(&self, reg: u16, v: u8) -> Result<()> {
        self.write_word_data(0x00, reg.swap_bytes())?;
        self.write_byte_data(0x01, v)
    }

    pub fn ene_write_block(&self, reg: u16, data: &[u8]) -> Result<()> {
        self.write_word_data(0x00, reg.swap_bytes())?;
        self.write_block_data(0x03, data)
    }

    /// ENE controllers: select a 16-bit register (byte-swapped) via cmd 0x00, then read it via cmd 0x81.
    pub fn ene_read(&self, reg: u16) -> Result<u8> {
        self.write_word_data(0x00, reg.swap_bytes())?;
        Ok(self.read_byte_data(0x81)?)
    }

    fn check_write(&self) -> Result<()> {
        if !self.writable { bail!("bus is read-only"); }
        match self.addr {
            Some(a) if is_protected(a) => bail!("refusing to write protected address {a:#04x}"),
            Some(_) => Ok(()),
            None => bail!("no address selected"),
        }
    }
}

/// Passive scan: receive-byte only, 0x08-0x77, skipping SPD/EEPROM space and addresses a kernel driver owns.
pub fn scan(num: u32) -> Result<Vec<(u16, &'static str)>> {
    let mut bus = Bus::open(num, false)?;
    let mut out = Vec::new();
    for addr in 0x08u16..=0x77 {
        if is_protected(addr) { continue; }
        if bus.set_addr(addr).is_err() { out.push((addr, "driver-owned")); continue; }
        if bus.read_byte().is_ok() { out.push((addr, "responds")); }
    }
    Ok(out)
}
