//! ENE DRAM RGB controllers (AUDA0-E6K5-0101, protocol V2) on SMBus.
use crate::i2c::{self, Bus};
use anyhow::{Result, bail};
use std::fs;

const FIRMWARE: &[u8] = b"AUDA0-E6K5-0101";

/// DDR5 modules whose SPD hub is bound to the kernel spd5118 driver at 0x50-0x57;
/// the matching RGB controller is expected at +0x20.
pub fn find_dram() -> Vec<(u32, u16)> {
    let mut out = Vec::new();
    for ad in i2c::adapters() {
        for spd in 0x50u16..=0x57 {
            let name = fs::read_to_string(format!("/sys/bus/i2c/devices/{}-{spd:04x}/name", ad.num));
            if name.is_ok_and(|n| n.trim() == "spd5118") { out.push((ad.num, spd + 0x20)); }
        }
    }
    out
}

/// Solid colour in direct mode. Volatile: the flash-save command is never issued.
pub fn set_color(bus_num: u32, addr: u16, [r, g, b]: [u8; 3]) -> Result<usize> {
    if !(0x70..=0x77).contains(&addr) { bail!("ene only drives 0x70-0x77"); }
    let mut bus = Bus::open(bus_num, true)?;
    bus.set_addr(addr)?;
    let id: Vec<u8> = (0..16u16).map(|i| bus.ene_read(0x1000 + i)).collect::<Result<_>>()?;
    if !id.starts_with(FIRMWARE) { bail!("unrecognised firmware {:?}; refusing to write", String::from_utf8_lossy(&id)); }
    let leds = bus.ene_read(0x1c02)?.max(bus.ene_read(0x1c03)?) as usize;
    if leds == 0 || leds > 15 { bail!("implausible LED count {leds}"); }
    bus.ene_write(0x8020, 1)?; // direct mode
    let data: Vec<u8> = (0..leds).flat_map(|_| [r, b, g]).collect(); // ENE wants R,B,G
    for (i, chunk) in data.chunks(15).enumerate() { bus.ene_write_block(0x8100 + (i * 15) as u16, chunk)?; }
    bus.ene_write(0x802f, 1)?; // apply
    Ok(leds)
}
