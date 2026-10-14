//! Complete native economics/history plus raw source; no full digest or whole-world Value copy.
use anyhow::{ensure, Result};
use kingmaker_core::{week::NativeWeekReceipt, WorldState};
use serde::{ser::SerializeStruct, Serialize, Serializer};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};
struct Receipt<'a>(&'a NativeWeekReceipt);
impl Serialize for Receipt<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let p = self.0;
        let mut out = serializer.serialize_struct("NativeWeekReceiptWithoutTimings", 11)?;
        macro_rules! field {
            ($name:ident) => {
                out.serialize_field(stringify!($name), &p.$name)?;
            };
        }
        field!(campaign_id);
        field!(request);
        field!(start_date);
        field!(end_date);
        field!(end_revision);
        field!(phase_order);
        field!(operation_evidence);
        field!(registry_evidence);
        field!(diagnostics);
        field!(invariants);
        field!(boundary);
        out.end()
    }
}
#[derive(Serialize)]
struct Weeks<'a> {
    phase_receipts: Vec<Receipt<'a>>,
}
struct State<'a>(&'a WorldState);
impl Serialize for State<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let w = self.0;
        let mut out = serializer.serialize_struct("EconomicWorldState", 15)?;
        out.serialize_field(
            "week_data",
            &Weeks {
                phase_receipts: w.week_data.phase_receipts.iter().map(Receipt).collect(),
            },
        )?;
        macro_rules! field {
            ($name:ident) => {
                out.serialize_field(stringify!($name), &w.$name)?;
            };
        }
        field!(construction_data);
        field!(income_data);
        field!(transport_data);
        field!(mobility_data);
        field!(labor_data);
        field!(budget_data);
        field!(market_data);
        field!(production_data);
        field!(campaign);
        field!(provinces);
        field!(pops);
        field!(enterprises);
        field!(stockpiles);
        field!(accounts);
        // account_index is reconstructed; source is streamed below, not omitted.
        out.end()
    }
}
pub fn write_economic_state(world: &WorldState, mut output: impl Write) -> Result<()> {
    serde_json::to_writer(&mut output, &State(world))?;
    output.write_all(b"\nSOURCE\n")?;
    if let Some(source) = &world.source {
        crate::checkpoint::write_source(&mut output, source)?;
    } else {
        output.write_all(b"null\n")?;
    }
    output.flush()?;
    Ok(())
}
pub fn save_economic_state(world: &WorldState, path: &Path) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut out = BufWriter::with_capacity(1024 * 1024, file);
    write_economic_state(world, &mut out)?;
    out.get_ref().sync_all()?;
    Ok(())
}
struct Compare {
    input: BufReader<File>,
    offset: u64,
    buffer: [u8; 65536],
}
impl Write for Compare {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for expected in bytes.chunks(self.buffer.len()) {
            self.input.read_exact(&mut self.buffer[..expected.len()])?;
            if self.buffer[..expected.len()] != *expected {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Economic/history/source differs near byte {}", self.offset),
                ));
            }
            self.offset += expected.len() as u64;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn compare_economic_state(world: &WorldState, path: &Path) -> Result<()> {
    let mut compare = Compare {
        input: BufReader::with_capacity(1024 * 1024, File::open(path)?),
        offset: 0,
        buffer: [0; 65536],
    };
    write_economic_state(world, &mut compare)?;
    let mut extra = [0u8; 1];
    ensure!(
        compare.input.read(&mut extra)? == 0,
        "Extra economic comparison data"
    );
    Ok(())
}
