use super::*;

const QUAD: &str = r#"#VRML V2.0 utf8
Transform {
  translation 1 2 3
  rotation 0 0 1 1.5707963267948966
  children [
    Shape {
      appearance Appearance { material Material { diffuseColor 0.8 0.2 0.1 } }
      geometry IndexedFaceSet {
        coord Coordinate { point [ 0 0 0, 2 0 0, 2 1 0, 0 1 0 ] }
        coordIndex [ 0, 1, 2, -1, 0, 2, 3, -1 ]
      }
    }
  ]
}
"#;

#[test]
fn reads_a_transformed_indexed_face_set() {
    let mesh = read_vrml(QUAD.as_bytes(), "part.wrl").unwrap();
    assert_eq!(mesh.triangles.len(), 2);
    assert_eq!(mesh.color, Some([0.8, 0.2, 0.1]));
    // (0,0,0) and (2,0,0) turn a quarter turn about Z, then move by (1,2,3).
    let find = |want: Vec3| mesh.positions.iter().any(|p| (*p - want).len() < 1e-9);
    assert!(find(Vec3::new(1.0, 2.0, 3.0)), "{:?}", mesh.positions);
    assert!(find(Vec3::new(1.0, 4.0, 3.0)), "{:?}", mesh.positions);
    assert_eq!(mesh.warnings, Vec::<String>::new());
}

#[test]
fn resolves_def_use_and_counts_primitives() {
    let source = r#"#VRML V2.0 utf8
DEF box Shape { geometry Box { size 2 2 2 } }
Group { children [ USE box, Transform { children [ USE box ] } ] }
"#;
    let mesh = read_vrml(source.as_bytes(), "boxes.wrl").unwrap();
    assert_eq!(mesh.triangles.len(), 36, "the DEF node and its two USE copies, six quads each");
}

#[test]
fn refuses_vrml_1_and_unterminated_files() {
    assert!(read_vrml(b"#VRML V1.0 ascii\nSeparator {}\n", "old.wrl").is_err());
    assert!(read_vrml(b"#VRML V2.0 utf8\nShape { geometry Box {", "bad.wrl").is_err());
    assert!(!is_vrml_path("part.step"));
    assert!(is_vrml_path("part.WRL"));
}

#[test]
fn refuses_excessively_nested_lists() {
    let nesting = MAX_DEPTH + 1;
    let mut source = String::from("#VRML V2.0 utf8\nShape { geometry Box { size ");
    source.push_str(&"[".repeat(nesting));
    source.push('1');
    source.push_str(&"]".repeat(nesting));
    source.push_str(" } }\n");

    let error = read_vrml(source.as_bytes(), "nested.wrl").unwrap_err();
    assert!(error.to_string().contains("nesting exceeds"), "{error}");
}

#[test]
fn refuses_a_model_without_geometry() {
    let error = read_vrml(b"#VRML V2.0 utf8\nGroup { children [ ] }\n", "empty.wrl").unwrap_err();
    assert!(error.to_string().contains("no geometry"));
}

/// A small connector with named materials and one pin placed four times, each copy under its own
/// turn and offset, the way exporters write repeated parts. `shared` picks `DEF` and `USE`, or
/// every copy written out.
fn four_pin_connector(shared: bool) -> String {
    let housing = "Material { diffuseColor 0.15 0.15 0.15 }";
    let metal = "Material { diffuseColor 0.8 0.8 0.75 shininess 0.4 }";
    let pin = "IndexedFaceSet {
        coord Coordinate { point [ -0.1 -0.1 0, 0.1 -0.1 0, 0.1 0.1 0, -0.1 0.1 0, -0.1 -0.1 1, 0.1 -0.1 1, 0.1 0.1 1, -0.1 0.1 1 ] }
        coordIndex [ 0 3 2 1 -1, 4 5 6 7 -1, 0 1 5 4 -1, 1 2 6 5 -1, 2 3 7 6 -1, 3 0 4 7 -1 ]
      }";
    let named = |name: &str, node: &str| if shared { format!("DEF {name} {node}") } else { node.to_string() };
    let mut source = String::from("#VRML V2.0 utf8\n");
    source.push_str(&format!("Shape {{ appearance Appearance {{ material {} }} geometry Box {{ size 4 1 1 }} }}\n", named("HOUSING", housing)));
    for (i, x) in [-1.5, -0.5, 0.5, 1.5].into_iter().enumerate() {
        let (material, geometry) = match (shared, i) {
            (true, 0) => (named("METAL", metal), named("PIN", pin)),
            (true, _) => ("USE METAL".to_string(), "USE PIN".to_string()),
            (false, _) => (metal.to_string(), pin.to_string()),
        };
        let turn = i as f64 * std::f64::consts::FRAC_PI_2;
        source.push_str(&format!(
            "Transform {{ translation {x} 0 -1 rotation 0 0 1 {turn} children [\n  Shape {{ appearance Appearance {{ material {material} }} geometry {geometry} }}\n] }}\n"
        ));
    }
    source
}

#[test]
fn a_model_that_reuses_a_part_reads_as_the_model_written_out() {
    let shared = four_pin_connector(true);
    assert_eq!(shared.matches("USE PIN").count(), 3, "{shared}");
    let mesh = read_vrml(shared.as_bytes(), "connector.wrl").unwrap();
    assert_eq!(mesh, read_vrml(four_pin_connector(false).as_bytes(), "connector.wrl").unwrap());
    // The housing's 6 quads come as 36 corners, and each pin adds its 8 points and 12 triangles.
    assert_eq!(mesh.positions.len(), 36 + 4 * 8);
    assert_eq!(mesh.triangles.len(), 12 + 4 * 12);
    assert_eq!(mesh.color, Some([0.15, 0.15, 0.15]));
    // Each copy of the pin's corner (0.1, -0.1, 1) turns by its own quarter turns about Z, then
    // moves by its own offset.
    for want in [Vec3::new(-1.4, -0.1, 0.0), Vec3::new(-0.4, 0.1, 0.0), Vec3::new(0.4, 0.1, 0.0), Vec3::new(1.4, -0.1, 0.0)] {
        assert!(mesh.positions.iter().any(|p| (*p - want).len() < 1e-9), "{want:?} in {:?}", mesh.positions);
    }
}

/// Nested `USE`s that double each level make a file of about 64 KB stand for about 512 MB, and
/// each added level doubles that again. The file is refused once what it stands for passes
/// 256 MB, four times the limit for a file, without copying the parts it reuses.
#[test]
fn refuses_nested_uses_that_double_each_level() {
    let mut source = String::from("#VRML V2.0 utf8\nShape { geometry Box { } }\n");
    source.push_str(&format!("DEF L0 WorldInfo {{ title \"{}\" }}\n", "x".repeat(64 << 10)));
    for level in 1..=12 {
        source.push_str(&format!("DEF L{level} Group {{ children [ USE L{0} USE L{0} ] }}\n", level - 1));
    }
    let error = read_vrml(source.as_bytes(), "doubling.wrl").unwrap_err();
    assert!(error.to_string().contains("reuses parts too often"), "{error}");
}

/// One large node used many times over, the shape of the file the importer audit found, is
/// refused the same way.
#[test]
fn refuses_one_large_node_used_many_times() {
    let mut source = format!("#VRML V2.0 utf8\nShape {{ geometry Box {{ }} }}\nDEF A WorldInfo {{ title \"{}\" }}\n", "x".repeat(1 << 20));
    source.push_str(&"USE A\n".repeat(300));
    let error = read_vrml(source.as_bytes(), "repeated.wrl").unwrap_err();
    assert!(error.to_string().contains("reuses parts too often"), "{error}");
}
