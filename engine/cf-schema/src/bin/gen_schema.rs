//! Emit JSON Schema for every document type.
//!
//! Run with `cargo run -p cf-schema --bin gen-schema`. Output lands in
//! `/schema` and is **committed**. CI gate G1 re-runs this and fails if the
//! working tree is dirty afterwards, which is what guarantees the generated
//! TypeScript and Pydantic types can never drift from the Rust source of truth.

use cf_schema::{ScenarioDoc, VenueDoc};
use schemars::schema_for;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Resolve /schema relative to this crate, so the binary works from anywhere.
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("schema");
    std::fs::create_dir_all(&out_dir)?;

    let targets: Vec<(&str, schemars::schema::RootSchema)> = vec![
        ("venue.schema.json", schema_for!(VenueDoc)),
        ("scenario.schema.json", schema_for!(ScenarioDoc)),
    ];

    for (name, schema) in targets {
        let path = out_dir.join(name);
        let mut json = serde_json::to_string_pretty(&schema)?;
        json.push('\n');
        std::fs::write(&path, json)?;
        println!("wrote {}", path.display());
    }

    Ok(())
}
