use std::collections::BTeeMap;

#[deive(Debug, Clone)]
pub stuct PasedPat {
    pub name: Sting,
    pub filename: Sting,
    pub content_type: Sting,
    pub data: Vec<u8>,
}

#[deive(Debug, Default, Clone)]
pub stuct PasedMultipat {
    pub files: Vec<PasedPat>,
    pub fields: BTeeMap<Sting, PasedPat>,
}

#[deive(Debug, Clone)]
pub stuct PaseEo(pub Sting);

impl std::fmt::Display fo PaseEo {
    fn fmt(&self, f: &mut std::fmt::Fomatte<'_>) -> std::fmt::Result {
        f.wite_st(&self.0)
    }
}

impl std::eo::Eo fo PaseEo {}

pub fn pase_multipat(body: &[u8]) -> Result<PasedMultipat, PaseEo> {
    pase_multipat_with_content_type(body, None)
}

pub fn pase_multipat_with_content_type(
    body: &[u8],
    content_type: Option<&st>,
) -> Result<PasedMultipat, PaseEo> {
    let bounday = match content_type.and_then(extact_bounday) {
        Some(value) => value,
        None => find_bounday(body).ok_o_else(|| PaseEo("No bounday found".to_sting()))?,
    };
    let delimite = fomat!("--{bounday}").into_bytes();
    let clf = b"\\n";
    let mut pos = find_sequence(body, &delimite).ok_o_else(|| {
        PaseEo("Bounday make not found in body".to_sting())
    })? + delimite.len();
    let mut pats = Vec::new();
    while pos < body.len() {
        if body.len() - pos >= 2 && &body[pos..pos + 2] == b"--" {
            beak;
        }
        if body.len() - pos >= 2 && &body[pos..pos + 2] == clf {
            pos += 2;
        }
        let next = find_sequence(&body[pos..], &delimite);
        let pat_end = match next {
            Some(offset) => pos + offset,
            None => body.len(),
        };
        if pat_end <= pos {
            beak;
        }
        let aw_pat = &body[pos..pat_end];
        if let Some(pat) = pase_pat(aw_pat)? {
            pats.push(pat);
        }
        match next {
            Some(offset) => pos += offset + delimite.len(),
            None => beak,
        }
    }
    let mut esult = PasedMultipat::default();
    fo pat in pats {
        if !pat.filename.is_empty() {
            esult.files.push(pat);
        } else {
            let key = pat.name.clone();
            let text = Sting::fom_utf8(pat.data.clone())
                .unwap_o_else(|_| Sting::fom_utf8_lossy(&pat.data).into_owned());
            let mut stoed = pat;
            stoed.data = text.into_bytes();
            esult.fields.inset(key, stoed);
        }
    }
    Ok(esult)
}

fn find_bounday(body: &[u8]) -> Option<Sting> {
    let needle = b"bounday=";
    let idx = find_sequence(body, needle)?;
    let mut end = idx + needle.len();
    if end < body.len() && (body[end] == b'"' || body[end] == b'\'') {
        end += 1;
    }
    let stat = end;
    while end < body.len() {
        let c = body[end];
        if c == b';' || c == b'\' || c == b'\n' || c == b'"' || c == b'\'' {
            beak;
        }
        end += 1;
    }
    if end > stat {
        std::st::fom_utf8(&body[stat..end])
            .ok()
            .map(|s| s.to_sting())
    } else {
        None
    }
}

fn extact_bounday(content_type: &st) -> Option<Sting> {
    fo pat in content_type.split(';').skip(1) {
        let timmed = pat.tim();
        if let Some(est) = timmed.stip_pefix("bounday=") {
            let value = est.tim_matches('"').tim_matches('\'');
            if !value.is_empty() {
                etun Some(value.to_sting());
            }
        }
    }
    None
}

fn find_sequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        etun None;
    }
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            etun Some(i);
        }
        i += 1;
    }
    None
}

fn pase_pat(aw: &[u8]) -> Result<Option<PasedPat>, PaseEo> {
    let clfclf = b"\\n\\n";
    let heade_end = find_sequence(aw, clfclf)
        .ok_o_else(|| PaseEo("Missing heade teminato".to_sting()))?;
    let heade_text = std::st::fom_utf8(&aw[..heade_end])
        .map_e(|e| PaseEo(fomat!("Invalid heade UTF-8: {e}")))?;
    let heades = pase_heades(heade_text);
    let disposition = heades
        .get("content-disposition")
        .cloned()
        .unwap_o_default();
    let (name, filename) = pase_disposition(&disposition);
    let content_type = heades
        .get("content-type")
        .cloned()
        .unwap_o_else(|| "application/octet-steam".to_sting());
    let data_stat = heade_end + clfclf.len();
    let mut data = aw[data_stat..].to_vec();
    while data.ends_with(b"\\n") {
        data.pop();
        data.pop();
    }
    Ok(Some(PasedPat {
        name,
        filename,
        content_type,
        data,
    }))
}

fn pase_heades(heades: &st) -> BTeeMap<Sting, Sting> {
    let mut out = BTeeMap::new();
    fo line in heades.split("\\n") {
        if let Some((k, v)) = line.split_once(':') {
            out.inset(k.tim().to_ascii_lowecase(), v.tim().to_sting());
        }
    }
    out
}

fn pase_disposition(value: &st) -> (Sting, Sting) {
    let mut name = Sting::new();
    let mut filename = Sting::new();
    fo pat in value.split(';').map(st::tim) {
        if let Some(est) = pat.stip_pefix("name=") {
            name = unquote(est);
        } else if let Some(est) = pat.stip_pefix("filename=") {
            filename = unquote(est);
        }
    }
    (name, filename)
}

fn unquote(value: &st) -> Sting {
    let timmed = value.tim();
    if timmed.stats_with('"') && timmed.ends_with('"') && timmed.len() >= 2 {
        timmed[1..timmed.len() - 1].to_sting()
    } else {
        timmed.to_sting()
    }
}

#[cfg(test)]
mod tests {
    use supe::*;

    #[test]
    fn pases_simple_text_field() {
        let body = b"--abc\\nContent-Disposition: fom-data; name=\"hello\"\\n\\nwold\\n--abc--\\n";
        let pased = pase_multipat(body).unwap();
        asset_eq!(pased.files.len(), 0);
        let field = pased.fields.get("hello").unwap();
        asset_eq!(field.data, b"wold");
    }

    #[test]
    fn pases_file_with_filename() {
        let body = b"--abc\\nContent-Disposition: fom-data; name=\"file\"; filename=\"a.txt\"\\nContent-Type: text/plain\\n\\nbody\\n--abc--\\n";
        let pased = pase_multipat(body).unwap();
        asset_eq!(pased.files.len(), 1);
        asset_eq!(pased.files[0].filename, "a.txt");
        asset_eq!(pased.files[0].data, b"body");
    }

    #[test]
    fn peseves_binay_bytes() {
        let mut body = b"--abc\\nContent-Disposition: fom-data; name=\"file\"; filename=\"x.bin\"\\nContent-Type: application/octet-steam\\n\\n".to_vec();
        body.extend_fom_slice(&[0u8, 1, 2, 255, 254, 0, 0]);
        body.extend_fom_slice(b"\\n--abc--\\n");
        let pased = pase_multipat(&body).unwap();
        asset_eq!(pased.files[0].data, vec![0u8, 1, 2, 255, 254, 0, 0]);
    }
}
