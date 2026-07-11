use std::str::FromStr;

use rgit_objects::{
    ActorId, AnyObject, CanonicalLimits, CanonicalObject, Chunk, HashAlgorithm, Manifest,
    ManifestEntry, ManifestTarget, ObjectId, PathSegment, PolicyId, PolicyRef, PortablePath,
    Signature, SignatureAlgorithm, SignatureError, SignaturePurpose, Value, decode_canonical,
};

fn zero_id() -> ObjectId {
    let mut bytes = vec![0, 0x12, 32];
    bytes.extend([0; 32]);
    ObjectId::from_bytes(&bytes).unwrap()
}
fn policy() -> PolicyRef {
    PolicyRef {
        policy_id: PolicyId::from_bytes([0; 16]),
        version: zero_id(),
    }
}

#[test]
fn published_chunk_vector_is_byte_stable_and_decodes() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("vectors/chunk-v0.json")).unwrap();
    let object = Chunk {
        policy_ref: policy(),
        bytes: b"abc".to_vec(),
    };
    let encoded = object.encode().unwrap();
    assert_eq!(hex::encode(&encoded), fixture["canonical_cbor_hex"]);
    assert_eq!(
        hex::encode(ObjectId::preimage(1, 0, &encoded)),
        fixture["preimage_hex"]
    );
    let id = object.id(HashAlgorithm::Sha256).unwrap();
    assert_eq!(hex::encode(id.to_bytes()), fixture["binary_id_hex"]);
    assert_eq!(id.to_string(), fixture["text_id"]);
    assert_eq!(ObjectId::from_str(&id.to_string()).unwrap(), id);
    let decoded = AnyObject::decode_verified(&encoded, &id, CanonicalLimits::default()).unwrap();
    assert_eq!(decoded.decoded().kind() as u64, 1);
}

#[test]
fn decoder_rejects_ambiguous_encodings_on_every_platform() {
    for bytes in [
        &[0x18, 0x17][..],
        &[0x9f, 0xff],
        &[0xf9, 0, 0],
        &[0xc0, 0],
        &[0xa2, 1, 0, 0, 0],
        &[0xa2, 0, 0, 0, 1],
    ] {
        assert!(
            decode_canonical(bytes, CanonicalLimits::default()).is_err(),
            "accepted {}",
            hex::encode(bytes)
        );
    }
    assert!(
        Value::Map(vec![(0, Value::Null), (0, Value::Null)])
            .encode()
            .is_err()
    );
    assert!(Value::Text("e\u{301}".into()).encode().is_err());
}

#[test]
fn integer_boundaries_have_fixed_big_endian_wire_bytes() {
    let cases = [
        (0, "00"),
        (23, "17"),
        (24, "1818"),
        (255, "18ff"),
        (256, "190100"),
        (65535, "19ffff"),
        (65536, "1a00010000"),
        (u64::from(u32::MAX), "1affffffff"),
        (u64::from(u32::MAX) + 1, "1b0000000100000000"),
        (u64::MAX, "1bffffffffffffffff"),
    ];
    for (number, expected) in cases {
        let encoded = Value::Unsigned(number).encode().unwrap();
        assert_eq!(hex::encode(&encoded), expected);
        assert_eq!(
            decode_canonical(&encoded, CanonicalLimits::default()).unwrap(),
            Value::Unsigned(number)
        );
    }

    for (number, expected) in [
        (-1, "20"),
        (-24, "37"),
        (-25, "3818"),
        (-256, "38ff"),
        (-257, "390100"),
        (-65_536, "39ffff"),
        (-65_537, "3a00010000"),
        (i64::MIN, "3b7fffffffffffffff"),
    ] {
        let encoded = Value::Signed(number).encode().unwrap();
        assert_eq!(hex::encode(&encoded), expected);
        assert_eq!(
            decode_canonical(&encoded, CanonicalLimits::default()).unwrap(),
            Value::Signed(number)
        );
    }
}

#[test]
fn canonical_limits_fail_before_unbounded_nesting_or_collections() {
    let limits = CanonicalLimits {
        max_bytes: 16,
        max_byte_string_bytes: 1,
        max_string_bytes: 1,
        max_depth: 1,
        max_collection_items: 1,
    };
    assert!(decode_canonical(&[0x42, 0, 0], limits).is_err());
    assert!(decode_canonical(&[0x62, b'a', b'b'], limits).is_err());
    assert!(decode_canonical(&[0x82, 0, 0], limits).is_err());
    assert!(decode_canonical(&[0x81, 0x81, 0xf6], limits).is_err());

    let tiny = CanonicalLimits {
        max_bytes: 1,
        ..CanonicalLimits::default()
    };
    assert!(decode_canonical(&[0x41, 0], tiny).is_err());
}

#[test]
fn encoder_enforces_each_limit_at_its_exact_boundary() {
    let limits = CanonicalLimits {
        max_bytes: 4,
        max_byte_string_bytes: 2,
        max_string_bytes: 2,
        max_depth: 1,
        max_collection_items: 1,
    };

    assert_eq!(
        Value::Bytes(vec![0, 1]).encode_with_limits(limits).unwrap(),
        [0x42, 0, 1]
    );
    assert!(matches!(
        Value::Bytes(vec![0, 1, 2]).encode_with_limits(limits),
        Err(rgit_objects::CanonicalError::SizeLimit)
    ));
    assert!(Value::Text("ab".into()).encode_with_limits(limits).is_ok());
    assert!(matches!(
        Value::Text("abc".into()).encode_with_limits(limits),
        Err(rgit_objects::CanonicalError::SizeLimit)
    ));
    assert!(
        Value::Array(vec![Value::Null])
            .encode_with_limits(limits)
            .is_ok()
    );
    assert!(matches!(
        Value::Array(vec![Value::Null, Value::Null]).encode_with_limits(limits),
        Err(rgit_objects::CanonicalError::CollectionLimit)
    ));
    assert!(
        Value::Array(vec![Value::Array(vec![])])
            .encode_with_limits(limits)
            .is_ok()
    );
    assert!(matches!(
        Value::Array(vec![Value::Array(vec![Value::Null])]).encode_with_limits(limits),
        Err(rgit_objects::CanonicalError::DepthLimit)
    ));

    let exact_size = CanonicalLimits {
        max_bytes: 3,
        ..limits
    };
    assert!(
        Value::Bytes(vec![0, 1])
            .encode_with_limits(exact_size)
            .is_ok()
    );
    assert!(matches!(
        Value::Bytes(vec![0, 1]).encode_with_limits(CanonicalLimits {
            max_bytes: 2,
            ..limits
        }),
        Err(rgit_objects::CanonicalError::SizeLimit)
    ));
}

#[test]
fn four_mibibyte_chunk_round_trips_only_under_bulk_profile() {
    let object = Chunk {
        policy_ref: policy(),
        bytes: vec![0x5a; 4 * 1024 * 1024],
    };
    let encoded = object.encode().unwrap();
    assert!(AnyObject::decode(&encoded, CanonicalLimits::metadata()).is_err());
    assert!(AnyObject::decode(&encoded, CanonicalLimits::bulk()).is_ok());

    let oversized = Chunk {
        policy_ref: policy(),
        bytes: vec![0; 4 * 1024 * 1024 + 1],
    };
    assert!(oversized.encode().is_err());
}

#[test]
fn path_segments_reject_traversal_and_nonportable_names() {
    for name in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
        assert!(PathSegment::new(name).is_err());
    }
    assert!(PathSegment::new("e\u{301}").is_err());
    for name in ["CON", "con.txt", "trailing.", "trailing "] {
        assert!(PathSegment::new_portable(name).is_err());
    }
    assert_eq!(PathSegment::new_portable("src").unwrap().as_str(), "src");
    assert!(PortablePath::new(vec![PathSegment::new("CON").unwrap()]).is_err());

    let manifest = Manifest {
        policy_ref: policy(),
        entries: vec![ManifestEntry {
            name: PathSegment::new("CON").unwrap(),
            target: ManifestTarget::Directory {
                manifest: zero_id(),
            },
            policy_ref: policy(),
        }],
    };
    assert!(manifest.encode().is_err());
}

#[test]
fn portable_paths_reject_windows_illegal_ascii_and_controls() {
    for character in ['<', '>', ':', '"', '|', '?', '*'] {
        let name = format!("before{character}after");
        assert!(
            PathSegment::new_portable(&name).is_err(),
            "accepted Windows-illegal character U+{:04X}",
            character as u32
        );
        assert_eq!(PathSegment::new(name).unwrap().as_str().len(), 12);
    }

    for codepoint in (0..=0x1f).chain(std::iter::once(0x7f)) {
        let character = char::from_u32(codepoint).unwrap();
        let name = format!("a{character}b");
        assert!(
            PathSegment::new_portable(name).is_err(),
            "accepted ASCII control U+{codepoint:04X}"
        );
    }
}

#[test]
fn portable_paths_reject_all_windows_device_aliases() {
    let stems = [
        "CON", "prn", "AuX", "nul", "COM1", "com9", "LPT1", "lpt9", "COM¹", "com²", "CoM³", "LPT¹",
        "lpt²", "LpT³",
    ];
    for stem in stems {
        for name in [
            stem.to_owned(),
            format!("{stem}.txt"),
            format!("{stem}.archive.tar"),
        ] {
            assert!(
                PathSegment::new_portable(&name).is_err(),
                "accepted reserved device name {name:?}"
            );
        }
    }

    for name in ["COM0", "COM10", "LPT0", "LPT10", "CONSOLE", "NUL-safe"] {
        assert!(
            PathSegment::new_portable(name).is_ok(),
            "rejected non-device name {name:?}"
        );
    }
}

#[test]
fn portable_paths_preserve_valid_unicode_and_scope_collisions_to_siblings() {
    for name in [
        "café.txt",
        "数据",
        "ファイル",
        "🦀.rs",
        "fullwidth：colon",
        "ＣＯＮ",
    ] {
        assert_eq!(PathSegment::new_portable(name).unwrap().as_str(), name);
    }

    let repeated = PortablePath::new(vec![
        PathSegment::new_portable("foo").unwrap(),
        PathSegment::new_portable("foo").unwrap(),
    ])
    .unwrap();
    assert_eq!(repeated.segments().len(), 2);

    // These manifests model `foo/foo`: the equal names occur in distinct
    // directories, so neither manifest has a sibling collision.
    let child = Manifest {
        policy_ref: policy(),
        entries: vec![ManifestEntry {
            name: PathSegment::new_portable("foo").unwrap(),
            target: ManifestTarget::File {
                blob: zero_id(),
                mode: rgit_objects::FileMode::Regular,
            },
            policy_ref: policy(),
        }],
    };
    let child_id = child.id(HashAlgorithm::Sha256).unwrap();
    let parent = Manifest {
        policy_ref: policy(),
        entries: vec![ManifestEntry {
            name: PathSegment::new_portable("foo").unwrap(),
            target: ManifestTarget::Directory { manifest: child_id },
            policy_ref: policy(),
        }],
    };
    assert!(child.encode().is_ok());
    assert!(parent.encode().is_ok());
}

#[test]
fn portable_manifest_encoder_decoder_round_trip() {
    let entries = vec![
        ManifestEntry {
            name: PathSegment::new_portable("数据").unwrap(),
            target: ManifestTarget::SecretRef {
                secret_ref: zero_id(),
            },
            policy_ref: policy(),
        },
        ManifestEntry {
            name: PathSegment::new_portable("bin").unwrap(),
            target: ManifestTarget::File {
                blob: zero_id(),
                mode: rgit_objects::FileMode::Executable,
            },
            policy_ref: policy(),
        },
        ManifestEntry {
            name: PathSegment::new_portable("src").unwrap(),
            target: ManifestTarget::Directory {
                manifest: zero_id(),
            },
            policy_ref: policy(),
        },
        ManifestEntry {
            name: PathSegment::new_portable("current").unwrap(),
            target: ManifestTarget::Symlink {
                target_blob: zero_id(),
            },
            policy_ref: policy(),
        },
        ManifestEntry {
            name: PathSegment::new_portable("vendor").unwrap(),
            target: ManifestTarget::Subproject {
                subproject: zero_id(),
            },
            policy_ref: policy(),
        },
    ];
    let manifest = Manifest {
        policy_ref: policy(),
        entries,
    };
    let encoded = manifest.encode().unwrap();
    let expected_value = decode_canonical(&encoded, CanonicalLimits::default()).unwrap();
    let decoded = AnyObject::decode(&encoded, CanonicalLimits::default()).unwrap();
    assert_eq!(decoded.decoded().value(), &expected_value);
    assert_eq!(decoded.decoded().value().encode().unwrap(), encoded);
    assert_eq!(
        decoded.id(HashAlgorithm::Sha256).unwrap(),
        manifest.id(HashAlgorithm::Sha256).unwrap()
    );
}

#[test]
fn default_case_folding_is_pinned_and_not_lowercase_approximation() {
    assert_eq!(unicode_casefold::UNICODE_VERSION, (9, 0, 0));
    assert_eq!(
        PathSegment::new_portable("Straße")
            .unwrap()
            .portable_case_fold(),
        "strasse"
    );
    assert_eq!(
        PathSegment::new_portable("ΟΣ")
            .unwrap()
            .portable_case_fold(),
        PathSegment::new_portable("οσ")
            .unwrap()
            .portable_case_fold()
    );

    for (left, right) in [("Straße", "STRASSE"), ("ΟΣ", "ος")] {
        let manifest = Manifest {
            policy_ref: policy(),
            entries: vec![
                ManifestEntry {
                    name: PathSegment::new_portable(left).unwrap(),
                    target: ManifestTarget::Directory {
                        manifest: zero_id(),
                    },
                    policy_ref: policy(),
                },
                ManifestEntry {
                    name: PathSegment::new_portable(right).unwrap(),
                    target: ManifestTarget::Directory {
                        manifest: zero_id(),
                    },
                    policy_ref: policy(),
                },
            ],
        };
        assert!(manifest.encode().is_err(), "accepted {left:?}/{right:?}");
    }
}

#[test]
fn signatures_require_nonempty_material_and_registered_purpose() {
    let actor = ActorId::from_bytes([7; 16]);
    assert_eq!(
        Signature::new(
            SignatureAlgorithm::Ed25519,
            actor,
            [0; 32],
            [1; 64],
            SignaturePurpose::Operation,
        ),
        Err(SignatureError::PlaceholderKeyId)
    );
    assert_eq!(
        Signature::new(
            SignatureAlgorithm::Ed25519,
            actor,
            [1; 32],
            [0; 64],
            SignaturePurpose::Operation,
        ),
        Err(SignatureError::PlaceholderSignature)
    );
    assert!(SignaturePurpose::try_from("user-controlled").is_err());
    for (purpose, text) in [
        (SignaturePurpose::LineState, "line-state"),
        (SignaturePurpose::Operation, "operation"),
        (SignaturePurpose::Marker, "marker"),
        (SignaturePurpose::Release, "release"),
        (SignaturePurpose::Policy, "policy"),
    ] {
        assert_eq!(purpose.as_str(), text);
        assert_eq!(SignaturePurpose::try_from(text).unwrap(), purpose);
        assert!(text.is_ascii());
    }
}

#[test]
fn both_hash_algorithms_are_distinct_and_round_trip() {
    let object = Chunk {
        policy_ref: policy(),
        bytes: vec![],
    };
    let sha = object.id(HashAlgorithm::Sha256).unwrap();
    let blake = object.id(HashAlgorithm::Blake3_256).unwrap();
    assert_ne!(sha, blake);
    assert_eq!(ObjectId::from_bytes(&sha.to_bytes()).unwrap(), sha);
    assert_eq!(ObjectId::from_bytes(&blake.to_bytes()).unwrap(), blake);
}

#[test]
fn debug_json_is_not_the_hashed_encoding() {
    let object = Chunk {
        policy_ref: policy(),
        bytes: b"abc".to_vec(),
    };
    let json = object.debug_json().unwrap();
    assert!(json.contains("hex:616263"));
    assert_ne!(json.as_bytes(), object.encode().unwrap());
}
