pub mod model;
pub mod ops;
pub mod parser;
pub mod undo;
pub mod writer;

use anyhow::{Context, Result};
use std::path::Path;

/// Read and parse a journal file.
pub fn load(path: &Path, commodity: Option<&str>) -> Result<(model::Journal, String)> {
  let text =
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
  let journal =
    parser::parse(&text, commodity).with_context(|| format!("parsing {}", path.display()))?;
  Ok((journal, text))
}
