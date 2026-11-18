//! Builds the product map of a small fixture that mirrors a Next.js app-router + FastAPI layout and compares it
//! with a committed snapshot. Regenerate with `MWA_UPDATE_SNAPSHOTS=1 cargo test -p mwa-productmap`.

use std::path::PathBuf;

use mwa_productmap::adapters;
use mwa_productmap::map::{BuildInput, build};

fn fixture(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mini")
        .join(p)
}

#[test]
fn mini_product_map_matches_snapshot() {
    let openapi = std::fs::read_to_string(fixture("openapi.json")).unwrap();
    let input = BuildInput {
        frontend: &fixture("frontend"),
        backend: &fixture("backend"),
        frontend_sha: None,
        backend_sha: None,
        openapi: Some(("fixtures/mini/openapi.json".into(), openapi)),
    };
    let adapter = adapters::load(&fixture("adapter.toml")).unwrap();
    let map = build(adapter, &input).unwrap();
    let got = serde_json::to_string_pretty(&map).unwrap() + "\n";
    let snap = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/mini.json");
    if std::env::var("MWA_UPDATE_SNAPSHOTS").is_ok() {
        std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
        std::fs::write(&snap, &got).unwrap();
    }
    let want =
        std::fs::read_to_string(&snap).expect("snapshot missing; run with MWA_UPDATE_SNAPSHOTS=1");
    assert!(
        got == want,
        "product map differs from tests/snapshots/mini.json; review the diff and regenerate"
    );
}
