use minase_trainer::nn::npz::{self, Array};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

fn encode(arrays: BTreeMap<String, Array>) -> Vec<u8> {
    let entries: Vec<_> = arrays.into_iter().collect();
    npz::encode(
        &entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn change_meta(arrays: &mut BTreeMap<String, Array>, change: impl FnOnce(&mut Value)) {
    let array = arrays.get_mut("meta").unwrap();
    let mut meta = serde_json::from_slice(&array.data).unwrap();
    change(&mut meta);
    array.data = serde_json::to_vec(&meta).unwrap();
    array.shape = vec![array.data.len()];
}

/// 指示書の契約に従い、正当なZIP内の種別・配列集合・dtype・論理形状の違いを拒否する。
pub fn rejects_invalid_schema(bytes: &[u8], accepts: impl Fn(&[u8]) -> bool) {
    let arrays = npz::decode(bytes).unwrap();
    let name = arrays
        .iter()
        .find(|(_, a)| a.dtype == "<f4" && a.shape.len() == 2)
        .unwrap()
        .0
        .clone();
    for case in [
        "kind",
        "missing",
        "extra",
        "dtype",
        "shape",
        "meta_dtype",
        "meta_shape",
    ] {
        let mut changed = arrays.clone();
        match case {
            "kind" => change_meta(&mut changed, |meta| {
                meta["kind"] = if meta["kind"] == "nnue" {
                    json!("pst")
                } else {
                    json!("nnue")
                }
            }),
            "missing" => {
                changed.remove(&name);
            }
            "extra" => {
                changed.insert("extra".into(), changed[&name].clone());
            }
            "dtype" => changed.get_mut(&name).unwrap().dtype = "<i4".into(),
            // 要素数を保ち、ZIP/NPYとして妥当なまま論理形状だけを変える。
            "shape" => changed.get_mut(&name).unwrap().shape.reverse(),
            "meta_dtype" => changed.get_mut("meta").unwrap().dtype = "|i1".into(),
            "meta_shape" => {
                let a = changed.get_mut("meta").unwrap();
                a.shape = vec![1, a.data.len()];
            }
            _ => unreachable!(),
        }
        assert!(!accepts(&encode(changed)), "accepted {case}");
    }
}

/// config.jsonを変えずmetaの各欄を変え、再開が欄名付きエラーになり保存物を更新しないことを確かめる。
pub fn rejects_changed_meta(directory: &Path, resume: impl Fn() -> String) {
    let path = directory.join("latest.npz");
    let original = fs::read(&path).unwrap();
    let document = fs::read(directory.join("config.json")).unwrap();
    let arrays = npz::decode(&original).unwrap();
    for field in ["settings", "inputs", "preprocessing"] {
        let mut changed = arrays.clone();
        change_meta(&mut changed, |meta| {
            meta["config"][field] = json!({"changed": true})
        });
        let changed = encode(changed);
        fs::write(&path, &changed).unwrap();
        let error = resume();
        assert!(error.contains(field), "{field}: {error}");
        assert_eq!(fs::read(&path).unwrap(), changed);
        assert_eq!(fs::read(directory.join("config.json")).unwrap(), document);
    }
    fs::write(path, original).unwrap();
}
