use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ParsedPart {
    pub name: String,
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Default, Clone)]
pub struct ParsedMultipart {
    pub files: Vec<ParsedPart>,
    pub fields: BTreeMap<String, ParsedPart>,
}

#[derive(Debug, Clone)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

pub fn parse_multipart(body: &[u8]) -> Result<ParsedMultipart, ParseError> {
    parse_multipart_with_content_type(body, None)
}

pub fn parse_multipart_with_content_type(
    body: &[u8],
    content_type: Option<&str>,
) -> Result<ParsedMultipart, ParseError> {
    let boundary = match content_type.and_then(extract_boundary) {
        Some(value) => value,
        None => find_boundary(body).ok_or_else(|| ParseError("No boundary found".to_string()))?,
    };
    let delimiter = format!("--{boundary}").into_bytes();
    let crlf = b"\r\n";
    let mut pos = find_sequence(body, &delimiter).ok_or_else(|| {
        ParseError("Boundary marker not found in body".to_string())
    })? + delimiter.len();
    let mut parts = Vec::new();
    while pos < body.len() {
        if body.len() - pos >= 2 && &body[pos..pos + 2] == b"--" {
            break;
        }
        if body.len() - pos >= 2 && &body[pos..pos + 2] == crlf {
            pos += 2;
        }
        let next = find_sequence(&body[pos..], &delimiter);
        let part_end = match next {
            Some(offset) => pos + offset,
            None => body.len(),
        };
        if part_end <= pos {
            break;
        }
        let raw_part = &body[pos..part_end];
        if let Some(part) = parse_part(raw_part)? {
            parts.push(part);
        }
        match next {
            Some(offset) => pos += offset + delimiter.len(),
            None => break,
        }
    }
    let mut result = ParsedMultipart::default();
    for part in parts {
        if !part.filename.is_empty() {
            result.files.push(part);
        } else {
            let key = part.name.clone();
            let text = String::from_utf8(part.data.clone())
                .unwrap_or_else(|_| String::from_utf8_lossy(&part.data).into_owned());
            let mut stored = part;
            stored.data = text.into_bytes();
            result.fields.insert(key, stored);
        }
    }
    Ok(result)
}

fn find_boundary(body: &[u8]) -> Option<String> {
    let needle = b"boundary=";
    let idx = find_sequence(body, needle)?;
    let mut end = idx + needle.len();
    if end < body.len() && (body[end] == b'"' || body[end] == b'\'') {
        end += 1;
    }
    let start = end;
    while end < body.len() {
        let c = body[end];
        if c == b';' || c == b'\r' || c == b'\n' || c == b'"' || c == b'\'' {
            break;
        }
        end += 1;
    }
    if end > start {
        std::str::from_utf8(&body[start..end])
            .ok()
            .map(|s| s.to_string())
    } else {
        None
    }
}

fn extract_boundary(content_type: &str) -> Option<String> {
    for part in content_type.split(';').skip(1) {
        let trimmed = part.trim();
        if let Some(rest) = trimmed.strip_prefix("boundary=") {
            let value = rest.trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn find_sequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_part(raw: &[u8]) -> Result<Option<ParsedPart>, ParseError> {
    let crlfcrlf = b"\r\n\r\n";
    let header_end = find_sequence(raw, crlfcrlf)
        .ok_or_else(|| ParseError("Missing header terminator".to_string()))?;
    let header_text = std::str::from_utf8(&raw[..header_end])
        .map_err(|err| ParseError(format!("Invalid header UTF-8: {err}")))?;
    let headers = parse_headers(header_text);
    let disposition = headers
        .get("content-disposition")
        .cloned()
        .unwrap_or_default();
    let (name, filename) = parse_disposition(&disposition);
    let content_type = headers
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let data_start = header_end + crlfcrlf.len();
    let mut data = raw[data_start..].to_vec();
    while data.ends_with(b"\r\n") {
        data.pop();
        data.pop();
    }
    Ok(Some(ParsedPart {
        name,
        filename,
        content_type,
        data,
    }))
}

fn parse_headers(headers: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in headers.split("\r\n") {
        if let Some((k, v)) = line.split_once(':') {
            out.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    out
}

fn parse_disposition(value: &str) -> (String, String) {
    let mut name = String::new();
    let mut filename = String::new();
    for part in value.split(';').map(str::trim) {
        if let Some(rest) = part.strip_prefix("name=") {
            name = unquote(rest);
        } else if let Some(rest) = part.strip_prefix("filename=") {
            filename = unquote(rest);
        }
    }
    (name, filename)
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_text_field() {
        let body = b"--abc\r\nContent-Disposition: form-data; name=\"hello\"\r\n\r\nworld\r\n--abc--\r\n";
        let parsed = parse_multipart(body).unwrap();
        assert_eq!(parsed.files.len(), 0);
        let field = parsed.fields.get("hello").unwrap();
        assert_eq!(field.data, b"world");
    }

    #[test]
    fn parses_file_with_filename() {
        let body = b"--abc\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nbody\r\n--abc--\r\n";
        let parsed = parse_multipart(body).unwrap();
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0].filename, "a.txt");
        assert_eq!(parsed.files[0].data, b"body");
    }

    #[test]
    fn preserves_binary_bytes() {
        let mut body = b"--abc\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
        body.extend_from_slice(&[0u8, 1, 2, 255, 254, 0, 0]);
        body.extend_from_slice(b"\r\n--abc--\r\n");
        let parsed = parse_multipart(&body).unwrap();
        assert_eq!(parsed.files[0].data, vec![0u8, 1, 2, 255, 254, 0, 0]);
    }
}
