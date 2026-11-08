use sha2::{Digest, Sha256};
use std::{fs, path::Path};

#[test]
fn bundled_product_artwork_matches_its_pinned_inventory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("branding.lock.json")).unwrap()).unwrap();
    assert_eq!(
        lock["repository"],
        "https://github.com/every-lemon/kagnode-logos"
    );
    let commit = lock["commit"].as_str().unwrap();
    assert_eq!(commit.len(), 40);
    assert!(commit.bytes().all(|b| b.is_ascii_hexdigit()));
    let files = lock["files"].as_array().unwrap();
    assert!(!files.is_empty());
    for asset in files {
        let path = Path::new(asset["path"].as_str().unwrap());
        assert!(path.starts_with("static"));
        assert!(
            path.components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
        );
        let bytes = fs::read(root.join(path)).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(bytes)),
            asset["sha256"],
            "{}",
            path.display()
        );
    }
    assert!(
        files
            .iter()
            .any(|a| a["source"] == "source/FONT-NOTICES.txt")
    );
    assert!(files.iter().any(|a| a["source"] == "LICENSE"));
}
