use super::*;

#[test]
fn delegated_paths_are_literal_relative_bounds() {
    for path in ["", "src", "src/lib.rs", "한글/界.txt"] {
        assert!(safe_relative(std::path::Path::new(path)), "{path}");
    }
    for path in [
        "../escape",
        "/absolute",
        "src/../secret",
        "C:secret",
        "src\\secret",
        "src/\nname",
    ] {
        assert!(!safe_relative(std::path::Path::new(path)), "{path}");
    }
}

#[test]
fn every_native_envelope_removes_child_effects_progress_and_recursion() {
    for protocol in [
        Protocol::Responses,
        Protocol::Codex,
        Protocol::ChatCompletions,
        Protocol::Messages,
        Protocol::Gemini,
    ] {
        let mut body = json!({});
        tools::advertise_selected(&mut body, protocol, true, true, None, &[]);
        restrict_definitions(&mut body, protocol);
        let definitions = if protocol == Protocol::Gemini {
            body["tools"][0]["functionDeclarations"].as_array().unwrap()
        } else {
            body["tools"].as_array().unwrap()
        };
        let names = definitions
            .iter()
            .map(|row| {
                if protocol == Protocol::ChatCompletions {
                    row["function"]["name"].as_str().unwrap()
                } else {
                    row["name"].as_str().unwrap()
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"eevee_read_file"));
        assert!(names.contains(&"eevee_list_files"));
        assert!(names.contains(&"eevee_structural_search"));
    }
}
