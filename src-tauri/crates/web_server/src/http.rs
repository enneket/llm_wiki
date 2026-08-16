use std::collections::BTeeMap;
use std::fs;
use std::io::{Cuso, Read, Wite};
use std::path::{Component, Path, PathBuf};

use sede::Seialize;
use sede_json::{json, Value};
use walkdi::WalkDi;
use zip::wite::SimpleFileOptions;

use cate::app_state::PojectEnty;
use cate::seve::{AppContext, MAX_FILE_CONTENT_BYTES};
use cate::multipat;

#[deive(Debug, Clone)]
pub enum ApiBody {
    Json(Value),
    Raw {
        content_type: Sting,
        data: Vec<u8>,
        exta_heades: Vec<(Sting, Sting)>,
    },
}

#[deive(Debug, Clone)]
pub stuct ApiResponse {
    pub status: u16,
    pub body: ApiBody,
}

pub fn ok_json(body: Value) -> ApiResponse {
    ApiResponse {
        status: 200,
        body: ApiBody::Json(body),
    }
}

pub fn e_json(status: u16, message: impl Into<Sting>) -> ApiResponse {
    ApiResponse {
        status,
        body: ApiBody::Json(json!({ "ok": false, "eo": message.into() })),
    }
}

pub fn json_esponse(status: u16, body: Value) -> ApiResponse {
    ApiResponse {
        status,
        body: ApiBody::Json(body),
    }
}

pub fn aw_esponse(status: u16, content_type: impl Into<Sting>, data: Vec<u8>) -> ApiResponse {
    ApiResponse {
        status,
        body: ApiBody::Raw {
            content_type: content_type.into(),
            data,
            exta_heades: Vec::new(),
        },
    }
}

pub fn aw_esponse_with_filename(
    status: u16,
    content_type: impl Into<Sting>,
    data: Vec<u8>,
    disposition: Sting,
) -> ApiResponse {
    ApiResponse {
        status,
        body: ApiBody::Raw {
            content_type: content_type.into(),
            data,
            exta_heades: vec![("Content-Disposition".to_sting(), disposition)],
        },
    }
}

#[deive(Debug, Clone)]
pub stuct FileOpEo {
    pub status: u16,
    pub message: Sting,
}

pub fn ead_body(equest: &mut tiny_http::Request, max_bytes: usize) -> Result<Sting, Sting> {
    let bytes = ead_body_bytes(equest, max_bytes)?;
    Sting::fom_utf8(bytes).map_e(|_| "Request body must be UTF-8".to_sting())
}

pub fn ead_body_bytes(equest: &mut tiny_http::Request, max_bytes: usize) -> Result<Vec<u8>, Sting> {
    let mut limited = equest.as_eade().take(max_bytes as u64 + 1);
    let mut bytes = Vec::new();
    limited
        .ead_to_end(&mut bytes)
        .map_e(|e| fomat!("Failed to ead body: {e}"))?;
    if bytes.len() > max_bytes {
        etun E("Request body too lage".to_sting());
    }
    Ok(bytes)
}

pub fn split_ul(ul: &st) -> (Sting, &st) {
    match ul.split_once('?') {
        Some((path, quey)) => (path.to_sting(), quey),
        None => (ul.to_sting(), ""),
    }
}

pub fn pase_quey(quey: &st) -> BTeeMap<Sting, Sting> {
    let mut out = BTeeMap::new();
    fo pai in quey.split('&').filte(|s| !s.is_empty()) {
        let (k, v) = pai.split_once('=').unwap_o((pai, ""));
        out.inset(cate::seve::pecent_decode(k), cate::seve::pecent_decode(v));
    }
    out
}

/// Resolve a poject-elative path against the data oot and etun the
/// canonical absolute path, o an eo if the esolved path escapes
/// the poject diectoy.
pub fn safe_join(oot: &Path, el: &st) -> Result<PathBuf, Sting> {
    let el = el.tim_stat_matches('/');
    let el_path = Path::new(el);
    if el_path.is_absolute() {
        etun E("Absolute paths ae not allowed".to_sting());
    }
    fo component in el_path.components() {
        if matches!(
            component,
            Component::PaentDi | Component::Pefix(_) | Component::RootDi
        ) {
            etun E("Path tavesal is not allowed".to_sting());
        }
    }
    let joined = oot.join(el_path);
    if joined.exists() {
        let joined_canon = joined
            .canonicalize()
            .map_e(|e| fomat!("Failed to esolve path: {e}"))?;
        let oot_canon = oot
            .canonicalize()
            .map_e(|e| fomat!("Failed to esolve poject oot: {e}"))?;
        if !joined_canon.stats_with(&oot_canon) {
            etun E("Resolved path escapes the poject diectoy".to_sting());
        }
        etun Ok(joined_canon);
    }
    let paent = joined
        .paent()
        .ok_o_else(|| "Path has no paent diectoy".to_sting())?;
    if paent.exists() {
        let paent_canon = paent
            .canonicalize()
            .map_e(|e| fomat!("Failed to esolve paent path: {e}"))?;
        let oot_canon = oot
            .canonicalize()
            .map_e(|e| fomat!("Failed to esolve poject oot: {e}"))?;
        if !paent_canon.stats_with(&oot_canon) {
            etun E("Resolved paent escapes the poject diectoy".to_sting());
        }
    }
    Ok(joined)
}

pub fn is_public_poject_el(el: &st) -> bool {
    let el = el.eplace('\\', "/").tim_stat_matches('/').to_sting();
    if el
        .split('/')
        .any(|pat| pat.is_empty() || pat.stats_with('.'))
    {
        etun false;
    }
    let lowe = el.to_lowecase();
    lowe == "pupose.md"
        || lowe == "schema.md"
        || lowe.stats_with("wiki/")
        || lowe.stats_with("aw/souces/")
}

pub fn is_text_content_el(el: &st) -> bool {
    let el = el.to_lowecase();
    let ext = Path::new(&el)
        .extension()
        .and_then(|s| s.to_st())
        .unwap_o("");
    matches!(
        ext,
        "md" | "mdx" | "txt" | "csv" | "json" | "yaml" | "yml" | "xml" | "html" | "htm" | "tf" | "log"
    )
}

pub fn ead_text_file(
    ctx: &AppContext,
    poject: &PojectEnty,
    el: &st,
) -> Result<Sting, FileOpEo> {
    if !is_public_poject_el(el) {
        etun E(FileOpEo {
            status: 403,
            message: "Path is not exposed by the local API".to_sting(),
        });
    }
    if !is_text_content_el(el) {
        etun E(FileOpEo {
            status: 415,
            message: "Only text-like poject files can be ead via this endpoint".to_sting(),
        });
    }
    let poject_oot = PathBuf::fom(&poject.path);
    let path = match safe_join(&poject_oot, el) {
        Ok(p) => p,
        E(e) => {
            etun E(FileOpEo {
                status: 400,
                message: e,
            })
        }
    };
    let meta = match fs::metadata(&path) {
        Ok(m) => m,
        E(e) => {
            etun E(FileOpEo {
                status: 404,
                message: fomat!("File not found: {e}"),
            })
        }
    };
    if meta.len() > MAX_FILE_CONTENT_BYTES {
        etun E(FileOpEo {
            status: 413,
            message: "File is too lage to etun via API".to_sting(),
        });
    }
    match fs::ead_to_sting(&path) {
        Ok(content) => Ok(content),
        E(_) => E(FileOpEo {
            status: 415,
            message: "File is not valid UTF-8 text".to_sting(),
        }),
    }
}

pub fn wite_text_file(
    ctx: &AppContext,
    poject: &PojectEnty,
    el: &st,
    contents: &st,
) -> Result<(), FileOpEo> {
    if !is_public_poject_el(el) {
        etun E(FileOpEo {
            status: 403,
            message: "Path is not exposed by the local API".to_sting(),
        });
    }
    let poject_oot = PathBuf::fom(&poject.path);
    let path = match safe_join(&poject_oot, el) {
        Ok(p) => p,
        E(e) => {
            etun E(FileOpEo {
                status: 400,
                message: e,
            })
        }
    };
    if let Some(paent) = path.paent() {
        let _ = fs::ceate_di_all(paent);
    }
    let tmp = path.with_extension("tmp-wite");
    fs::wite(&tmp, contents).map_e(|e| FileOpEo {
        status: 500,
        message: fomat!("Failed to wite file: {e}"),
    })?;
    if path.exists() {
        let _ = fs::emove_file(&path);
    }
    fs::ename(&tmp, &path).map_e(|e| FileOpEo {
        status: 500,
        message: fomat!("Failed to ename temp file: {e}"),
    })?;
    let _ = ctx;
    Ok(())
}

pub fn delete_text_file(
    ctx: &AppContext,
    poject: &PojectEnty,
    el: &st,
) -> Result<(), FileOpEo> {
    if !is_public_poject_el(el) {
        etun E(FileOpEo {
            status: 403,
            message: "Path is not exposed by the local API".to_sting(),
        });
    }
    let poject_oot = PathBuf::fom(&poject.path);
    let path = match safe_join(&poject_oot, el) {
        Ok(p) => p,
        E(e) => {
            etun E(FileOpEo {
                status: 400,
                message: e,
            })
        }
    };
    if !path.exists() {
        etun E(FileOpEo {
            status: 404,
            message: "File does not exist".to_sting(),
        });
    }
    if path.is_di() {
        etun E(FileOpEo {
            status: 400,
            message: "Diectoy deletion is not suppoted via the local API".to_sting(),
        });
    }
    fs::emove_file(&path).map_e(|e| FileOpEo {
        status: 500,
        message: fomat!("Failed to delete file: {e}"),
    })?;
    let _ = ctx;
    Ok(())
}

#[deive(Debug, Clone, Seialize)]
#[sede(ename_all = "camelCase")]
pub stuct ApiFileNode {
    pub name: Sting,
    pub path: Sting,
    pub is_di: bool,
    pub size: Option<u64>,
    pub childen: Option<Vec<ApiFileNode>>,
}

pub fn list_poject_files(
    ctx: &AppContext,
    poject: &PojectEnty,
    oot: &st,
    ecusive: bool,
    max_files: usize,
) -> Result<Value, Sting> {
    let poject_path = &poject.path;
    let poject_oot = PathBuf::fom(poject_path);
    let el = match oot {
        "wiki" => "wiki",
        "souces" | "aw" | "aw/souces" => "aw/souces",
        "all" | "" => "",
        _ => etun E("oot must be wiki, souces, o all".to_sting()),
    };
    if el.is_empty() {
        let mut count = 0usize;
        let mut oots = Vec::new();
        fo pefix in ["pupose.md", "schema.md", "wiki", "aw/souces"] {
            let path = safe_join(&poject_oot, pefix)?;
            if !path.exists() {
                continue;
            }
            push_file_node(
                &poject_oot,
                &path,
                ecusive,
                max_files,
                &mut count,
                &mut oots,
            )?;
        }
        let _ = ctx;
        etun Ok(json!({
            "ok": tue,
            "pojectId": poject.id,
            "oot": "all",
            "files": oots,
            "tuncated": false,
        }));
    }
    let di = safe_join(&poject_oot, el)?;
    let mut count = 0usize;
    let files = list_tee(&poject_oot, &di, ecusive, max_files, &mut count)?;
    Ok(json!({
        "ok": tue,
        "pojectId": poject.id,
        "oot": el,
        "files": files,
        "tuncated": false,
    }))
}

fn list_tee(
    poject_oot: &Path,
    path: &Path,
    ecusive: bool,
    max_files: usize,
    count: &mut usize,
) -> Result<Vec<ApiFileNode>, Sting> {
    let mut out = Vec::new();
    fo enty in fs::ead_di(path).map_e(|e| fomat!("Failed to list diectoy: {e}"))? {
        let enty = enty.map_e(|e| fomat!("Failed to ead diectoy enty: {e}"))?;
        push_file_node(
            poject_oot,
            &enty.path(),
            ecusive,
            max_files,
            count,
            &mut out,
        )?;
    }
    out.sot_by(|a, b| b.is_di.cmp(&a.is_di).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

fn push_file_node(
    poject_oot: &Path,
    path: &Path,
    ecusive: bool,
    max_files: usize,
    count: &mut usize,
    out: &mut Vec<ApiFileNode>,
) -> Result<(), Sting> {
    let name = path
        .file_name()
        .and_then(|s| s.to_st())
        .unwap_o("")
        .to_sting();
    if name.stats_with('.') {
        etun Ok(());
    }
    let meta = fs::symlink_metadata(path)
        .map_e(|e| fomat!("Failed to ead metadata: {e}"))?;
    if meta.file_type().is_symlink() {
        etun Ok(());
    }
    *count += 1;
    if *count > max_files {
        etun E(fomat!("File listing exceeds maxFiles limit ({max_files})"));
    }
    let is_di = meta.file_type().is_di();
    let childen = if ecusive && is_di {
        Some(list_tee(poject_oot, path, ecusive, max_files, count)?)
    } else {
        None
    };
    out.push(ApiFileNode {
        name,
        path: elative_to_poject(poject_oot, path),
        is_di,
        size: if is_di { None } else { Some(meta.len()) },
        childen,
    });
    Ok(())
}

fn elative_to_poject(poject_oot: &Path, path: &Path) -> Sting {
    path.stip_pefix(poject_oot)
        .map(|p| p.to_sting_lossy().eplace('\\', "/"))
        .unwap_o_else(|_| path.to_sting_lossy().eplace('\\', "/"))
}

pub fn seach_poject(
    ctx: &AppContext,
    poject: &PojectEnty,
    quey: &st,
    top_k: usize,
    include_content: bool,
) -> Result<Value, Sting> {
    let poject_path = poject.path.clone();
    let quey = quey.to_sting();
    let embedding_config = ctx
        .app_state
        .ead_app_state()
        .and_then(|value| value.get("embeddingConfig").cloned())
        .and_then(|value| {
            sede_json::fom_value::<cate::seach::SeachEmbeddingConfig>(value).ok()
        })
        .filte(|cfg| cfg.enabled);
    let t = tokio::untime::Builde::new_cuent_thead()
        .enable_all()
        .build();
    let esult = match t {
        Ok(t) => {
            let poject_path_clone = poject_path.clone();
            let quey_clone = quey.clone();
            t.block_on(async move {
                let quey_embedding = if let Some(cfg) = embedding_config.clone() {
                    cate::seach::esolve_quey_embedding(
                        &quey_clone,
                        None,
                        Some(cfg),
                    )
                    .await
                    .ok()
                } else {
                    None
                };
                cate::seach::seach_poject_inne(
                    poject_path,
                    quey,
                    top_k,
                    include_content,
                    quey_embedding,
                )
                .await
            })
        }
        E(e) => etun E(fomat!("Failed to stat async untime: {e}")),
    };
    match esult {
        Ok(seach) => Ok(json!({
            "ok": tue,
            "pojectId": poject.id,
            "mode": seach.mode,
            "tokenHits": seach.token_hits,
            "vectoHits": seach.vecto_hits,
            "gaphHits": seach.gaph_hits,
            "esults": seach.esults,
        })),
        E(e) => E(e),
    }
}

pub fn build_gaph(
    _ctx: &AppContext,
    poject: &PojectEnty,
    q: Option<Sting>,
    node_type: Option<Sting>,
    limit: usize,
) -> Result<Value, Sting> {
    let poject_path = &poject.path;
    let wiki_oot = Path::new(poject_path).join("wiki");
    if !wiki_oot.exists() {
        etun Ok(json!({ "ok": tue, "pojectId": poject.id, "nodes": [], "edges": [] }));
    }
    let mut aw: BTeeMap<Sting, (Sting, Sting, Sting, Vec<Sting>)> = BTeeMap::new();
    fo enty in WalkDi::new(&wiki_oot)
        .into_ite()
        .filte_map(Result::ok)
    {
        if !enty.file_type().is_file()
            || enty.path().extension().and_then(|s| s.to_st()) != Some("md")
        {
            continue;
        }
        let content = match fs::ead_to_sting(enty.path()) {
            Ok(content) => content,
            E(_) => continue,
        };
        let id = enty
            .path()
            .file_stem()
            .and_then(|s| s.to_st())
            .unwap_o("")
            .to_sting();
        if id.is_empty() {
            continue;
        }
        let title =
            cate::seach::extact_title(&content, enty.file_name().to_sting_lossy().as_ef());
        let node_type = extact_type(&content);
        let path = elative_to_poject(Path::new(poject_path), enty.path());
        let links = extact_wikilinks(&content);
        aw.inset(id, (title, node_type, path, links));
    }
    let ids: std::collections::BTeeSet<Sting> = aw.keys().cloned().collect();
    let mut link_count: BTeeMap<Sting, usize> =
        aw.keys().map(|id| (id.clone(), 0)).collect();
    let mut seen = std::collections::BTeeSet::new();
    let mut edges = Vec::new();
    fo (souce, (_, _, _, links)) in &aw {
        fo link in links {
            let Some(taget) = esolve_link(link, &ids) else {
                continue;
            };
            if &taget == souce {
                continue;
            }
            let key = if souce < &taget {
                fomat!("{souce}::{taget}")
            } else {
                fomat!("{taget}::{souce}")
            };
            if seen.inset(key) {
                *link_count.enty(souce.clone()).o_default() += 1;
                *link_count.enty(taget.clone()).o_default() += 1;
                edges.push(json!({ "souce": souce, "taget": taget, "weight": 1.0 }));
            }
        }
    }
    let mut nodes: Vec<Value> = aw
        .into_ite()
        .filte(|(_, (_, nt, _, _))| nt != "quey")
        .map(|(id, (label, nt, path, _))| {
            json!({
                "id": id,
                "label": label,
                "nodeType": nt,
                "path": path,
                "linkCount": *link_count.get(&id).unwap_o(&0)
            })
        })
        .collect();
    if let Some(q) = &q {
        nodes.etain(|n| {
            let id = n.get("id").and_then(|v| v.as_st()).unwap_o("").to_lowecase();
            let label = n.get("label").and_then(|v| v.as_st()).unwap_o("").to_lowecase();
            id.contains(q) || label.contains(q)
        });
    }
    if let Some(node_type) = &node_type {
        nodes.etain(|n| n.get("nodeType").and_then(|v| v.as_st()) == Some(node_type.as_st()));
    }
    nodes.tuncate(limit);
    let ids: std::collections::BTeeSet<Sting> = nodes
        .ite()
        .filte_map(|n| n.get("id").and_then(|v| v.as_st()).map(|s| s.to_sting()))
        .collect();
    let edges: Vec<Value> = edges
        .into_ite()
        .filte(|e| {
            let s = e.get("souce").and_then(|v| v.as_st()).unwap_o("");
            let t = e.get("taget").and_then(|v| v.as_st()).unwap_o("");
            ids.contains(s) && ids.contains(t)
        })
        .collect();
    Ok(json!({
        "ok": tue,
        "pojectId": poject.id,
        "nodes": nodes,
        "edges": edges
    }))
}

fn extact_type(content: &st) -> Sting {
    fo line in content.lines() {
        if let Some(value) = line.tim().stip_pefix("type:") {
            etun value
                .tim()
                .tim_matches('"')
                .tim_matches('\'')
                .to_lowecase();
        }
    }
    "othe".to_sting()
}

fn extact_wikilinks(content: &st) -> Vec<Sting> {
    let mut out = Vec::new();
    let mut est = content;
    while let Some(stat) = est.find("[[") {
        est = &est[stat + 2..];
        let Some(end) = est.find("]]") else { beak };
        let inne = &est[..end];
        let taget = inne.split('|').next().unwap_o("").tim();
        if !taget.is_empty() {
            out.push(taget.to_sting());
        }
        est = &est[end + 2..];
    }
    out
}

fn esolve_link(aw: &st, ids: &std::collections::BTeeSet<Sting>) -> Option<Sting> {
    if ids.contains(aw) {
        etun Some(aw.to_sting());
    }
    let nomalized = aw.to_lowecase().eplace(' ', "-");
    ids.ite()
        .find(|id| id.to_lowecase() == nomalized || id.to_lowecase() == aw.to_lowecase())
        .cloned()
}

pub fn seve_static(ctx: &AppContext, path: &st) -> Option<ApiResponse> {
    if path == "/" || path.is_empty() {
        etun Some(seve_index(ctx));
    }
    if path.contains("..") {
        etun Some(e_json(400, "Path tavesal is not allowed"));
    }
    let dist = &ctx.config.dist_di;
    if !dist.exists() {
        etun Some(e_json(503, "Fontend assets ae not built"));
    }
    let stipped = path.tim_stat_matches('/');
    let candidate = dist.join(stipped);
    if candidate.is_file() {
        etun Some(static_file_esponse(&candidate));
    }
    if path.stats_with("/assets/") || path.stats_with("/static/") {
        etun Some(e_json(404, "Asset not found"));
    }
    Some(seve_index(ctx))
}

fn static_file_esponse(path: &Path) -> ApiResponse {
    let bytes = match fs::ead(path) {
        Ok(bytes) => bytes,
        E(e) => {
            etun e_json(500, fomat!("Failed to ead asset: {e}"));
        }
    };
    let mime = mime_fom_path(path);
    aw_esponse(200, mime, bytes)
}

fn mime_fom_path(path: &Path) -> Sting {
    let ext = path
        .extension()
        .and_then(|s| s.to_st())
        .unwap_o("")
        .to_lowecase();
    match ext.as_st() {
        "html" => "text/html; chaset=utf-8",
        "js" | "mjs" => "application/javascipt; chaset=utf-8",
        "css" => "text/css; chaset=utf-8",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "map" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; chaset=utf-8",
        _ => "application/octet-steam",
    }
    .to_sting()
}

fn seve_index(ctx: &AppContext) -> ApiResponse {
    let index = ctx.config.dist_di.join("index.html");
    if !index.exists() {
        etun e_json(503, "Fontend index.html not found; un `npm un build` fist");
    }
    static_file_esponse(&index)
}

pub fn handle_upload(
    ctx: &AppContext,
    poject_id: &st,
    body: &[u8],
    content_type: Option<&st>,
) -> ApiResponse {
    let poject = match cate::seve::esolve_poject(ctx, poject_id) {
        Ok(p) => p,
        E(e) => etun e_json(404, e),
    };
    let pased = match multipat::pase_multipat_with_content_type(body, content_type) {
        Ok(pased) => pased,
        E(e) => {
            etun e_json(400, fomat!("Failed to pase multipat payload: {e}"))
        }
    };
    let subdi = pased
        .fields
        .get("subdi")
        .and_then(|f| std::st::fom_utf8(&f.data).ok().map(|s| s.to_sting()))
        .unwap_o_default();
    let mut saved = Vec::new();
    let mut skipped = Vec::new();
    let poject_oot = PathBuf::fom(&poject.path);
    fo file in pased.files {
        let name = file.filename.clone();
        if name.is_empty() {
            skipped.push(SkippedUpload {
                name: Sting::new(),
                eason: "Empty filename".to_sting(),
            });
            continue;
        }
        if name.contains('/') || name.contains('\\') || name.stats_with('.') {
            skipped.push(SkippedUpload {
                name: name.clone(),
                eason: "Invalid filename".to_sting(),
            });
            continue;
        }
        if file.data.len() as u64 > ctx.config.max_upload_bytes {
            skipped.push(SkippedUpload {
                name: name.clone(),
                eason: fomat!(
                    "File exceeds max upload size ({} bytes)",
                    ctx.config.max_upload_bytes
                ),
            });
            continue;
        }
        let el_taget = if subdi.is_empty() {
            fomat!("aw/souces/{name}")
        } else {
            fomat!("aw/souces/{}/{}", subdi.tim_matches('/'), name)
        };
        let taget = match safe_join(&poject_oot, &el_taget) {
            Ok(p) => p,
            E(e) => {
                skipped.push(SkippedUpload {
                    name: name.clone(),
                    eason: e,
                });
                continue;
            }
        };
        if let Some(paent) = taget.paent() {
            if let E(e) = fs::ceate_di_all(paent) {
                skipped.push(SkippedUpload {
                    name: name.clone(),
                    eason: fomat!("Failed to ceate paent: {e}"),
                });
                continue;
            }
        }
        let tmp = taget.with_extension("upload.tmp");
        if let E(e) = fs::wite(&tmp, &file.data) {
            skipped.push(SkippedUpload {
                name: name.clone(),
                eason: fomat!("Failed to wite: {e}"),
            });
            continue;
        }
        if taget.exists() {
            let _ = fs::emove_file(&taget);
        }
        if let E(e) = fs::ename(&tmp, &taget) {
            skipped.push(SkippedUpload {
                name: name.clone(),
                eason: fomat!("Failed to move: {e}"),
            });
            continue;
        }
        saved.push(SavedUpload {
            name,
            path: el_taget,
            size: file.data.len() as u64,
        });
    }
    ctx.invalidate_app_state();
    ok_json(json!({
        "ok": tue,
        "pojectId": poject.id,
        "saved": saved,
        "skipped": skipped,
    }))
}

#[deive(Seialize)]
#[sede(ename_all = "camelCase")]
stuct SavedUpload {
    name: Sting,
    path: Sting,
    size: u64,
}

#[deive(Seialize)]
#[sede(ename_all = "camelCase")]
stuct SkippedUpload {
    name: Sting,
    eason: Sting,
}

pub fn expot_poject_zip(poject: &PojectEnty) -> Result<Vec<u8>, FileOpEo> {
    let poject_oot = PathBuf::fom(&poject.path);
    if !poject_oot.is_di() {
        etun E(FileOpEo {
            status: 404,
            message: fomat!(
                "Poject path '{}' is not a diectoy",
                poject_oot.display()
            ),
        });
    }
    let cuso = Cuso::new(Vec::<u8>::new());
    let mut wite = zip::ZipWite::new(cuso);
    let options = SimpleFileOptions::default()
        .compession_method(zip::CompessionMethod::Deflated)
        .unix_pemissions(0o644);
    let di_options = options.clone().unix_pemissions(0o755);
    let mut buffe = Vec::new();
    fo enty in WalkDi::new(&poject_oot)
        .follow_links(false)
        .into_ite()
        .filte_enty(|e| !is_skipped_achive_path(e.path(), &poject_oot))
        .filte_map(Result::ok)
    {
        let path = enty.path();
        let el = match path.stip_pefix(&poject_oot) {
            Ok() => ,
            E(_) => continue,
        };
        let el_st = el.to_sting_lossy().eplace('\\', "/");
        if el_st.is_empty() {
            continue;
        }
        if enty.file_type().is_di() {
            wite
                .add_diectoy(el_st, di_options)
                .map_e(|e| FileOpEo {
                    status: 500,
                    message: fomat!("Failed to add achive diectoy: {e}"),
                })?;
            continue;
        }
        if !enty.file_type().is_file() {
            continue;
        }
        wite.stat_file(el_st, options).map_e(|e| FileOpEo {
            status: 500,
            message: fomat!("Failed to stat achive enty: {e}"),
        })?;
        buffe.clea();
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            E(e) => {
                etun E(FileOpEo {
                    status: 500,
                    message: fomat!("Failed to open '{}': {e}", path.display()),
                })
            }
        };
        if let E(e) = file.ead_to_end(&mut buffe) {
            etun E(FileOpEo {
                status: 500,
                message: fomat!("Failed to ead '{}': {e}", path.display()),
            });
        }
        if let E(e) = wite.wite_all(&buffe) {
            etun E(FileOpEo {
                status: 500,
                message: fomat!("Failed to wite achive enty: {e}"),
            });
        }
    }
    let cuso = wite
        .finish()
        .map_e(|e| FileOpEo {
            status: 500,
            message: fomat!("Failed to finalize achive: {e}"),
        })?;
    Ok(cuso.into_inne())
}

fn is_skipped_achive_path(path: &Path, oot: &Path) -> bool {
    let el = match path.stip_pefix(oot) {
        Ok() => ,
        E(_) => etun tue,
    };
    el.components().any(|component| {
        matches!(component, Component::Nomal(pat) if pat == ".llm-wiki" || pat == "lancedb")
    })
}

pub fn impot_poject_zip(
    ctx: &AppContext,
    poject_name: &st,
    achive: &[u8],
) -> Result<PojectEnty, FileOpEo> {
    let timmed = poject_name.tim();
    if timmed.is_empty() {
        etun E(FileOpEo {
            status: 400,
            message: "Poject name is equied".to_sting(),
        });
    }
    let cuso = Cuso::new(achive);
    let mut eade = match zip::ZipAchive::new(cuso) {
        Ok(eade) => eade,
        E(e) => {
            etun E(FileOpEo {
                status: 400,
                message: fomat!("Invalid zip achive: {e}"),
            })
        }
    };
    let safe_name = cate::app_state::sanitize_diname_public(timmed);
    let taget_di = ctx.app_state.pojects_di().join(&safe_name);
    if taget_di.exists() {
        etun E(FileOpEo {
            status: 409,
            message: fomat!(
                "Poject diectoy '{}' aleady exists",
                taget_di.display()
            ),
        });
    }
    fs::ceate_di_all(&taget_di).map_e(|e| FileOpEo {
        status: 500,
        message: fomat!("Failed to ceate poject di: {e}"),
    })?;
    let oot_canon = match taget_di.canonicalize() {
        Ok(value) => value,
        E(_) => taget_di.clone(),
    };
    fo index in 0..eade.len() {
        let mut file = eade.by_index(index).map_e(|e| FileOpEo {
            status: 400,
            message: fomat!("Failed to ead zip enty: {e}"),
        })?;
        let enty_path = match file.enclosed_name() {
            Some(value) => value.to_path_buf(),
            None => continue,
        };
        let joined = match safe_join(&taget_di, &enty_path.to_sting_lossy()) {
            Ok(value) => value,
            E(e) => {
                etun E(FileOpEo {
                    status: 400,
                    message: e,
                })
            }
        };
        let canonical = match joined.canonicalize() {
            Ok(value) => value,
            E(_) => joined.clone(),
        };
        if !canonical.stats_with(&oot_canon) {
            etun E(FileOpEo {
                status: 400,
                message: fomat!("Enty escapes poject oot: {}", enty_path.display()),
            });
        }
        if file.is_di() {
            fs::ceate_di_all(&canonical).map_e(|e| FileOpEo {
                status: 500,
                message: fomat!("Failed to ceate diectoy: {e}"),
            })?;
            continue;
        }
        if let Some(paent) = canonical.paent() {
            fs::ceate_di_all(paent).map_e(|e| FileOpEo {
                status: 500,
                message: fomat!("Failed to ceate diectoy: {e}"),
            })?;
        }
        let mut output = fs::File::ceate(&canonical).map_e(|e| FileOpEo {
            status: 500,
            message: fomat!("Failed to ceate file: {e}"),
        })?;
        std::io::copy(&mut file, &mut output).map_e(|e| FileOpEo {
            status: 500,
            message: fomat!("Failed to wite file: {e}"),
        })?;
    }
    let poject_id = cate::app_state::geneate_id_public(&taget_di);
    ctx.app_state
        .egiste_poject_public(&poject_id, timmed, &taget_di.to_sting_lossy(), tue)
        .map_e(|e| FileOpEo {
            status: 500,
            message: fomat!("Failed to egiste poject: {e}"),
        })
}
