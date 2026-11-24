use super::*;

fn error(bytes: &[u8]) -> FormatError {
    match Network::decode(bytes) {
        Ok(_) => panic!("accepted invalid bytes"),
        Err(e) => e,
    }
}
fn layer_error(p: LayerStackParameters) -> FormatError {
    match LayerStackNetwork::new(p) {
        Ok(_) => panic!("accepted invalid parameters"),
        Err(e) => e,
    }
}
fn simple_error(p: SimpleParameters) -> FormatError {
    match SimpleNetwork::new(p) {
        Ok(_) => panic!("accepted invalid parameters"),
        Err(e) => e,
    }
}
fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn replace_arch(bytes: &[u8], arch: &str) -> Vec<u8> {
    let mut result = bytes[..8].to_vec();
    result.extend((arch.len() as u32).to_le_bytes());
    result.extend(arch.as_bytes());
    result.extend(&bytes[12 + u32_at(bytes, 8) as usize..]);
    result
}

#[test]
fn roundtrips_and_dispatch_for_both_structures() {
    for original in [
        Network::LayerStack(fixed_layer_stack()),
        Network::Simple(fixed_simple()),
    ] {
        let bytes = original.encode();
        let path = std::env::temp_dir().join(format!(
            "minase-phase2a-{}-{:?}.bin",
            std::process::id(),
            original.feature_set()
        ));
        std::fs::write(&path, &bytes).unwrap();
        let read = std::fs::read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let loaded = Network::decode(&read).unwrap();
        assert_eq!(original.feature_set(), loaded.feature_set());
        assert_eq!(original.width(), loaded.width());
        assert_eq!(original.fv_scale(), loaded.fv_scale());
        assert_eq!(loaded.encode(), bytes);
        match (&original, &loaded) {
            (Network::LayerStack(_), Network::LayerStack(_))
            | (Network::Simple(_), Network::Simple(_)) => (),
            _ => panic!("wrong dispatch"),
        }
        for p in specimens() {
            assert_eq!(original.evaluate(&p), loaded.evaluate(&p));
            for side in Color::ALL {
                assert_eq!(
                    original.evaluate_for(&p, side),
                    loaded.evaluate_for(&p, side)
                );
            }
        }
    }
}

#[test]
fn header_rejections_distinguish_all_causes() {
    for net in [
        Network::LayerStack(fixed_layer_stack()),
        Network::Simple(fixed_simple()),
    ] {
        let bytes = net.encode();
        let mutate = |offset, value| {
            let mut b = bytes.clone();
            put(&mut b, offset, value);
            error(&b)
        };
        assert!(matches!(mutate(0, 0), FormatError::Version { .. }));
        for length in [0, 16385, u32::MAX] {
            assert!(matches!(mutate(8, length), FormatError::ArchLength { .. }));
        }
        assert!(matches!(
            mutate(4, 0),
            FormatError::Hash {
                section: "network_hash",
                ..
            }
        ));
        let mut b = bytes.clone();
        b[12] = 255;
        assert_eq!(error(&b), FormatError::ArchUtf8);
        b = bytes.clone();
        b.extend([0, 1, 2]);
        assert_eq!(error(&b), FormatError::TrailingBytes { count: 3 });
        let a = arch(net.feature_set(), net.width(), 128);
        for name in [
            "HalfKP",
            if net.feature_set() == FeatureSet::A {
                "HalfKPVm"
            } else {
                "A"
            },
        ] {
            let wrong = a.replacen(net.feature_set().spec().arch_name(), name, 1);
            assert!(matches!(
                error(&replace_arch(&bytes, &wrong)),
                FormatError::FeatureName { .. }
            ));
        }
        for w in ["0", "1", "3", "1026", "-2", "184467440737095516160"] {
            let wrong = a.replace(&format!("->{}x2", net.width()), &format!("->{w}x2"));
            assert!(
                matches!(
                    error(&replace_arch(&bytes, &wrong)),
                    FormatError::Width { .. }
                ),
                "{w}"
            );
        }
        for w in ["2", "032", "+32"] {
            let wrong = a.replace(&format!("->{}x2", net.width()), &format!("->{w}x2"));
            assert!(
                matches!(
                    error(&replace_arch(&bytes, &wrong)),
                    FormatError::Architecture { .. }
                ),
                "{w}"
            );
        }
        for scale in ["0", "-1", "2147483648", "99999999999999999999999"] {
            let wrong = a.replace("fv_scale=128", &format!("fv_scale={scale}"));
            assert!(matches!(
                error(&replace_arch(&bytes, &wrong)),
                FormatError::FvScale { .. }
            ));
        }
        for wrong in [
            a.replace("fv_scale=128", "fv_scale=012"),
            a.replace("fv_scale=128", "fv_scale=+128"),
            a.replace(",fv_scale=128", ""),
            a.replace("Friend", "Enemy"),
            a.replace("Network=", "Net="),
            a.replace(&net.feature_set().spec().dimensions().to_string(), "1"),
        ] {
            assert!(matches!(
                error(&replace_arch(&bytes, &wrong)),
                FormatError::Architecture { .. }
            ));
        }
    }
}

fn small_layer() -> LayerStackParameters {
    let mut p = layer_stack_parameters();
    p.width = 2;
    p.ft_biases = vec![0; 2];
    p.ft_weights = vec![0; FEATURE_COUNT * 2];
    p.l1_weights = vec![0; 32];
    p
}
fn small_simple() -> SimpleParameters {
    SimpleParameters {
        width: 2,
        fv_scale: 12,
        ft_biases: vec![0; 2],
        ft_weights: vec![0; 964224 * 2],
        l1_biases: vec![0; 32],
        l1_weights: vec![0; 128],
        l2_biases: vec![0; 32],
        l2_weights: vec![0; 1024],
        output_bias: 0,
        output_weights: vec![0; 32],
    }
}

/// 形式の区画の始点と終点を指示書のバイト数から独立に数える。
fn sections(bytes: &[u8], layer: bool, width: usize) -> Vec<(&'static str, usize, usize)> {
    let mut sections = vec![
        ("version", 0, 4),
        ("network_hash", 4, 8),
        ("architecture length", 8, 12),
    ];
    let mut offset = 12;
    let mut add = |name, length| {
        let start = offset;
        offset += length;
        sections.push((name, start, offset));
    };
    add("architecture", u32_at(bytes, 8) as usize);
    if layer {
        add("buckets", 4);
    }
    add("ft_hash", 4);
    if layer {
        for name in ["FT biases", "FT weights"] {
            let length_at = sections.last().unwrap().2 + 17;
            let start = sections.last().unwrap().2;
            let size = u32_at(bytes, length_at) as usize;
            sections.push((name, start, start + 17));
            sections.push((name, start + 17, start + 21));
            sections.push((name, start + 21, start + 21 + size));
        }
        offset = sections.last().unwrap().2;
    } else {
        add("FT biases", 2 * width);
        add("FT weights", 2 * 964224 * width);
    }
    let mut add = |name, length| {
        let start = offset;
        offset += length;
        sections.push((name, start, offset));
    };
    if layer {
        add("PSQT bias", 4);
        add("PSQT weights", 4 * FEATURE_COUNT);
    }
    add("fc_hash", 4);
    add("L1 biases", 4 * if layer { 16 } else { 32 });
    add(
        "L1 weights",
        if layer {
            16 * width.next_multiple_of(32)
        } else {
            32 * (2 * width).next_multiple_of(32)
        },
    );
    add("L2 biases", 128);
    add("L2 weights", 1024);
    add("output bias", 4);
    add("output weights", 32);
    assert_eq!(offset, bytes.len());
    sections
}

#[test]
fn truncation_at_every_section_and_hash_padding_psqt_rejections() {
    for (net, layer) in [
        (
            Network::LayerStack(LayerStackNetwork::new(small_layer()).unwrap()),
            true,
        ),
        (
            Network::Simple(SimpleNetwork::new(small_simple()).unwrap()),
            false,
        ),
    ] {
        let bytes = net.encode();
        let layout = sections(&bytes, layer, 2);
        for &(section, start, end) in &layout {
            for cut in [start, (start + end) / 2, end - 1] {
                assert_eq!(
                    error(&bytes[..cut]),
                    FormatError::Truncated { section },
                    "cut={cut}"
                );
            }
        }
        for &(name, start, _) in &layout {
            if ["ft_hash", "fc_hash"].contains(&name) {
                let mut b = bytes.clone();
                put(&mut b, start, 0);
                assert!(matches!(error(&b),FormatError::Hash { section, .. } if section==name));
            }
            if name == "buckets" {
                let mut b = bytes.clone();
                put(&mut b, start, 2);
                assert_eq!(error(&b), FormatError::Buckets { actual: 2 });
            }
            if name == "PSQT bias" {
                let mut b = bytes.clone();
                put(&mut b, start, 1);
                assert_eq!(error(&b), FormatError::PsqtBias { value: 1 });
            }
            if name == "PSQT weights" {
                let mut b = bytes.clone();
                put(&mut b, start, (1 << 22) + 1);
                assert_eq!(
                    error(&b),
                    FormatError::PsqtRange {
                        row: 0,
                        value: (1 << 22) + 1
                    }
                );
                let mut b = bytes.clone();
                put(&mut b, start, 1);
                assert_eq!(error(&b), FormatError::MirrorPsqt { row: 0 });
            }
            if ["L1 biases", "L2 biases", "output bias"].contains(&name) {
                for value in [(1 << 24) + 1, -(1 << 24) - 1, i32::MIN] {
                    let mut b = bytes.clone();
                    put(&mut b, start, value as u32);
                    assert!(matches!(error(&b),FormatError::BiasRange { value:v, .. } if v==value));
                }
            }
            if name == "L1 weights" || (name == "L2 weights" && layer) {
                let column = if name == "L2 weights" {
                    30
                } else if layer {
                    2
                } else {
                    4
                };
                let mut b = bytes.clone();
                b[start + column] = 1;
                assert_eq!(
                    error(&b),
                    FormatError::Padding {
                        layer: name,
                        row: 0,
                        column
                    }
                );
            }
        }
    }
}

fn leb_block(data: &[u8]) -> Vec<u8> {
    let mut bytes = b"COMPRESSED_LEB128".to_vec();
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    bytes
}
fn leb_error(data: &[u8], count: usize) -> FormatError {
    leb128::decode(&mut Reader::new(&leb_block(data)), count, "test").unwrap_err()
}
#[test]
fn leb128_exact_encoding_and_malformed_blocks() {
    let values = [0, 1, -1, 63, 64, -64, -65, 32767, -32768];
    let expected = [
        0, 1, 0x7f, 0x3f, 0xc0, 0, 0x40, 0xbf, 0x7f, 0xff, 0xff, 1, 0x80, 0x80, 0x7e,
    ];
    let mut bytes = Vec::new();
    leb128::encode(&mut bytes, &values);
    assert_eq!(bytes, leb_block(&expected));
    assert_eq!(
        leb128::decode(&mut Reader::new(&bytes), values.len(), "test").unwrap(),
        values
    );
    for cut in [0, 16, 17, 20, 21, bytes.len() - 1] {
        assert_eq!(
            leb128::decode(&mut Reader::new(&bytes[..cut]), values.len(), "test").unwrap_err(),
            FormatError::Truncated { section: "test" }
        );
    }
    bytes[0] = 0;
    assert_eq!(
        leb128::decode(&mut Reader::new(&bytes), values.len(), "test").unwrap_err(),
        FormatError::LebMagic { section: "test" }
    );
    for (data, count) in [(vec![], 1), (vec![0], 2), (vec![0; 4], 1)] {
        assert!(matches!(
            leb_error(&data, count),
            FormatError::LebSize { .. }
        ));
    }
    for data in [[0x80, 0x80, 2], [0xff, 0xff, 0x7d]] {
        assert!(matches!(leb_error(&data, 1), FormatError::LebRange { .. }));
    }
    assert!(matches!(
        leb_error(&[0x80; 12], 4),
        FormatError::LebOverflow { .. }
    ));
    let mut invalid = [0x80; 12];
    invalid[9] = 2;
    assert!(matches!(
        leb_error(&invalid, 4),
        FormatError::LebOverflow { .. }
    ));
    assert_eq!(
        leb_error(&[0x80], 1),
        FormatError::Truncated { section: "test" }
    );
    assert_eq!(
        leb_error(&[0x80, 0], 2),
        FormatError::LebCount {
            section: "test",
            expected: 2,
            actual: 1
        }
    );
    assert_eq!(
        leb_error(&[0, 0], 1),
        FormatError::LebTrailing {
            section: "test",
            count: 1
        }
    );
    // 10バイトで終わるi64の値自体は復号でき、i16の範囲外として拒否する。
    assert!(matches!(
        leb_error(
            &[
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0
            ],
            4
        ),
        FormatError::LebRange { .. }
    ));
}

#[test]
fn constructor_rejects_lengths_ranges_and_mirror_violations() {
    for width in [0, 1, 3, 1026, usize::MAX] {
        let mut p = small_layer();
        p.width = width;
        assert!(matches!(layer_error(p), FormatError::Width { .. }));
    }
    for scale in [0, -1, i32::MIN] {
        let mut p = small_layer();
        p.fv_scale = scale;
        assert!(matches!(layer_error(p), FormatError::FvScale { .. }));
    }
    for field in [
        "ft_biases",
        "ft_weights",
        "psqt_weights",
        "l1_biases",
        "l1_weights",
        "l2_biases",
        "l2_weights",
        "output_weights",
    ] {
        let mut p = small_layer();
        match field {
            "ft_biases" => {
                p.ft_biases.pop();
            }
            "ft_weights" => {
                p.ft_weights.pop();
            }
            "psqt_weights" => {
                p.psqt_weights.pop();
            }
            "l1_biases" => {
                p.l1_biases.pop();
            }
            "l1_weights" => {
                p.l1_weights.pop();
            }
            "l2_biases" => {
                p.l2_biases.pop();
            }
            "l2_weights" => {
                p.l2_weights.pop();
            }
            "output_weights" => {
                p.output_weights.pop();
            }
            _ => unreachable!(),
        }
        assert!(matches!(layer_error(p),FormatError::ParameterLength {field:f,..} if f==field));
    }
    let mut p = small_layer();
    p.ft_biases[0] = 1;
    assert_eq!(layer_error(p), FormatError::MirrorBias { component: 0 });
    let mut p = small_layer();
    p.ft_weights[0] = 1;
    assert!(matches!(layer_error(p), FormatError::MirrorWeight { .. }));
    let mut p = small_layer();
    p.psqt_weights[0] = 1;
    assert_eq!(layer_error(p), FormatError::MirrorPsqt { row: 0 });
    let mut p = small_layer();
    p.psqt_bias = 1;
    assert_eq!(layer_error(p), FormatError::PsqtBias { value: 1 });
    let mut p = small_layer();
    p.psqt_weights[0] = i32::MIN;
    assert!(matches!(layer_error(p), FormatError::PsqtRange { .. }));
    let mut p = small_layer();
    p.ft_biases.fill(i16::MIN);
    assert_eq!(
        layer_error(p),
        FormatError::AccumulatorBound {
            bucket: 0,
            component: 0,
            bound: 32768
        }
    );
    // 各升で最大224、先獅子で224、バイアス287なら上限32767。1だけ増やすと拒否する。
    let mut p = small_layer();
    p.ft_weights.fill(224);
    p.ft_biases.fill(287);
    assert!(LayerStackNetwork::new(p).is_ok());
    let mut p = small_layer();
    p.ft_weights.fill(224);
    p.ft_biases.fill(288);
    assert_eq!(
        layer_error(p),
        FormatError::AccumulatorBound {
            bucket: 0,
            component: 0,
            bound: 32768
        }
    );
    let mut p = small_simple();
    p.ft_weights.fill(224);
    p.ft_biases.fill(287);
    assert!(SimpleNetwork::new(p).is_ok());
    let mut p = small_simple();
    p.ft_weights.fill(224);
    p.ft_biases.fill(288);
    assert_eq!(
        simple_error(p),
        FormatError::AccumulatorBound {
            bucket: 0,
            component: 0,
            bound: 32768
        }
    );
    // 最後の区分の先獅子も上限の検査に含む。
    let mut p = small_simple();
    p.ft_weights[964224 * 2 - 2] = i16::MIN;
    assert_eq!(
        simple_error(p),
        FormatError::AccumulatorBound {
            bucket: 71,
            component: 0,
            bound: 32768
        }
    );
    let mut p = small_simple();
    p.ft_weights.pop();
    assert!(matches!(
        simple_error(p),
        FormatError::ParameterLength {
            field: "ft_weights",
            ..
        }
    ));
}

/// 妥当なファイルのLEB128区画だけを置き換え、外側の並びを維持する。
fn replace_leb(bytes: &[u8], section: &str, payload: &[u8]) -> Vec<u8> {
    let layout = sections(bytes, true, 2);
    let start = layout
        .iter()
        .find(|&&(name, _, _)| name == section)
        .unwrap()
        .1;
    let old_end = start + 21 + u32_at(bytes, start + 17) as usize;
    let mut changed = bytes[..start].to_vec();
    changed.extend(leb_block(payload));
    changed.extend(&bytes[old_end..]);
    changed
}

#[test]
fn file_loading_enforces_leb_ranges_accumulator_bounds_and_mirror_sharing() {
    let bytes = LayerStackNetwork::new(small_layer()).unwrap().encode();
    let layout = sections(&bytes, true, 2);
    for section in ["FT biases", "FT weights"] {
        let start = layout
            .iter()
            .find(|&&(name, _, _)| name == section)
            .unwrap()
            .1;
        let count = if section == "FT biases" {
            2
        } else {
            FEATURE_COUNT * 2
        };
        let mut b = bytes.clone();
        b[start] = 0;
        assert_eq!(error(&b), FormatError::LebMagic { section });
        for size in [count - 1, count * 3 + 1] {
            let mut b = bytes.clone();
            put(&mut b, start + 17, size as u32);
            assert_eq!(
                error(&b),
                FormatError::LebSize {
                    section,
                    actual: size,
                    count
                }
            );
        }
        let mut payload = vec![0; count];
        payload.splice(0..1, [0x80, 0x80, 2]);
        assert_eq!(
            error(&replace_leb(&bytes, section, &payload)),
            FormatError::LebRange {
                section,
                value: 32768
            }
        );
        let mut payload = vec![0; count];
        payload[count - 1] = 0x80;
        assert_eq!(
            error(&replace_leb(&bytes, section, &payload)),
            FormatError::Truncated { section }
        );
        let mut payload = vec![0; count];
        payload.splice(0..2, [0x80, 0]);
        assert_eq!(
            error(&replace_leb(&bytes, section, &payload)),
            FormatError::LebCount {
                section,
                expected: count,
                actual: count - 1
            }
        );
        let payload = vec![0; count + 1];
        assert_eq!(
            error(&replace_leb(&bytes, section, &payload)),
            FormatError::LebTrailing { section, count: 1 }
        );
        let mut payload = vec![0; count];
        payload[0] = 1;
        let e = error(&replace_leb(&bytes, section, &payload));
        if section == "FT biases" {
            assert_eq!(e, FormatError::MirrorBias { component: 0 });
        } else {
            assert!(matches!(e, FormatError::MirrorWeight { .. }));
        }
    }
    let mut payload = vec![0; FEATURE_COUNT * 2];
    payload.splice(0..1, [0x80; 10]);
    assert_eq!(
        error(&replace_leb(&bytes, "FT weights", &payload)),
        FormatError::LebOverflow {
            section: "FT weights"
        }
    );
    let mut payload = vec![0; FEATURE_COUNT * 2];
    payload.splice(0..1, [0x80, 0x80, 0x7e]);
    assert_eq!(
        error(&replace_leb(&bytes, "FT weights", &payload)),
        FormatError::AccumulatorBound {
            bucket: 0,
            component: 0,
            bound: 32768
        }
    );

    // 検査を通る上限32767のファイルから、バイアスだけ1増やして32768にする。
    let mut p = small_layer();
    p.ft_weights.fill(224);
    p.ft_biases.fill(287);
    let bytes = LayerStackNetwork::new(p).unwrap().encode();
    // 288の符号付きLEB128は0xa0, 0x02。
    assert_eq!(
        error(&replace_leb(&bytes, "FT biases", &[0xa0, 2, 0xa0, 2])),
        FormatError::AccumulatorBound {
            bucket: 0,
            component: 0,
            bound: 32768
        }
    );
    let mut p = small_simple();
    p.ft_weights.fill(224);
    p.ft_biases.fill(287);
    let mut bytes = SimpleNetwork::new(p).unwrap().encode();
    let ft_bias = 12 + u32_at(&bytes, 8) as usize + 4;
    bytes[ft_bias..ft_bias + 2].copy_from_slice(&288_i16.to_le_bytes());
    assert_eq!(
        error(&bytes),
        FormatError::AccumulatorBound {
            bucket: 0,
            component: 0,
            bound: 32768
        }
    );
}

#[test]
fn simple_constructor_validates_every_parameter_and_extreme_valid_values() {
    for field in [
        "ft_biases",
        "ft_weights",
        "l1_biases",
        "l1_weights",
        "l2_biases",
        "l2_weights",
        "output_weights",
    ] {
        let mut p = small_simple();
        match field {
            "ft_biases" => {
                p.ft_biases.push(0);
            }
            "ft_weights" => {
                p.ft_weights.push(0);
            }
            "l1_biases" => {
                p.l1_biases.push(0);
            }
            "l1_weights" => {
                p.l1_weights.push(0);
            }
            "l2_biases" => {
                p.l2_biases.push(0);
            }
            "l2_weights" => {
                p.l2_weights.push(0);
            }
            "output_weights" => {
                p.output_weights.push(0);
            }
            _ => unreachable!(),
        }
        assert!(matches!(simple_error(p),FormatError::ParameterLength {field:f,..} if f==field));
    }
    for width in [0, 1, 3, 1025, usize::MAX] {
        let mut p = small_simple();
        p.width = width;
        assert!(matches!(simple_error(p), FormatError::Width { .. }));
    }
    for scale in [0, -1, i32::MIN] {
        let mut p = small_simple();
        p.fv_scale = scale;
        assert!(matches!(simple_error(p), FormatError::FvScale { .. }));
    }
    for layer in ["L1", "L2", "output"] {
        let mut p = small_simple();
        match layer {
            "L1" => p.l1_biases[0] = (1 << 24) + 1,
            "L2" => p.l2_biases[0] = (1 << 24) + 1,
            "output" => p.output_bias = (1 << 24) + 1,
            _ => unreachable!(),
        }
        assert_eq!(
            simple_error(p),
            FormatError::BiasRange {
                layer,
                value: (1 << 24) + 1
            }
        );
    }
    // 幅上限と除数上限の受理も固定し、上限の取り違えを検出する。
    assert!(check_width(1024).is_ok());
    assert!(check_scale(i32::MAX).is_ok());
    let mut p = small_layer();
    p.width = 1024;
    p.ft_biases = vec![0; 1024];
    p.ft_weights = vec![0; FEATURE_COUNT * 1024];
    p.l1_weights = vec![127; 16 * 1024];
    p.fv_scale = i32::MAX;
    p.l1_biases.fill(1 << 24);
    p.l2_biases.fill(-(1 << 24));
    p.output_bias = 1 << 24;
    p.psqt_weights.fill(1 << 22);
    let net = LayerStackNetwork::new(p).unwrap();
    assert_eq!(net.evaluate(&Position::initial()), 0);
    let mut p = small_simple();
    p.fv_scale = i32::MAX;
    p.l1_biases.fill(-(1 << 24));
    p.l2_biases.fill(1 << 24);
    p.output_bias = -(1 << 24);
    let net = SimpleNetwork::new(p).unwrap();
    assert_eq!(net.evaluate(&Position::initial()).unwrap(), 0);
}

#[test]
fn malformed_arbitrary_bytes_never_panic() {
    let mut rng = XorShift64::new(NonZeroU64::new(0x1bad).unwrap());
    for length in 0..128 {
        let mut bytes: Vec<_> = (0..length).map(|_| rng.next() as u8).collect();
        assert!(Network::decode(&bytes).is_err());
        if length >= 12 {
            put(&mut bytes, 0, 0x7AF3_2F21);
            put(&mut bytes, 8, (length - 12) as u32);
            assert!(Network::decode(&bytes).is_err());
            put(&mut bytes, 0, 0x7AF3_2F16);
            assert!(Network::decode(&bytes).is_err());
        }
    }
}
