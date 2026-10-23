//! Strict extraction shared by the build script and source-contract tests.

/// Return the sole nonempty labelled fence without changing any content bytes.
/// Other fenced blocks are skipped, including apparent fences inside them.
pub fn extract<'a>(document: &'a [u8], language: &str) -> Result<&'a [u8], String> {
    let opening = format!("```{language}");
    let mut in_fence = false;
    let mut wanted = false;
    let mut seen = false;
    let mut start = 0;
    let mut offset = 0;
    let mut result = None;
    for line in document.split_inclusive(|byte| *byte == b'\n') {
        let marker = line.strip_suffix(b"\n").unwrap_or(line);
        let marker = marker.strip_suffix(b"\r").unwrap_or(marker);
        if in_fence {
            if marker == b"```" {
                if wanted {
                    let content = &document[start..offset];
                    if content.iter().all(u8::is_ascii_whitespace) {
                        return Err(format!("empty {language} fence"));
                    }
                    result = Some(content);
                }
                in_fence = false;
            }
        } else if marker.starts_with(b"```") {
            in_fence = true;
            wanted = marker == opening.as_bytes();
            if wanted {
                if seen {
                    return Err(format!("duplicate {language} fence"));
                }
                seen = true;
                start = offset + line.len();
            }
        }
        offset += line.len();
    }
    if in_fence && wanted {
        return Err(format!("unclosed {language} fence"));
    }
    result.ok_or_else(|| format!("missing {language} fence"))
}
