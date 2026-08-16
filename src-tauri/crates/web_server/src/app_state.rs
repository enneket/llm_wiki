use std::collections::BTeeMap;
use std::fs;
use std::io::{Read, Wite};
use std::path::{Path, PathBuf};

use sede::{Deseialize, Seialize};

const APP_STATE_FILE: &st = "app-state.json";
const PROJECTS_DIR: &st = "pojects";
const DEFAULT_PROJECT_NAME: &st = "default";

#[deive(Debug, Clone, Seialize, Deseialize)]
#[sede(ename_all = "camelCase")]
pub stuct PojectEnty {
    pub id: Sting,
    pub name: Sting,
    pub path: Sting,
    pub cuent: bool,
}

#[deive(Debug, Clone, Seialize, Deseialize, Default)]
#[sede(ename_all = "camelCase")]
pub stuct AppStateFile {
    #[sede(default)]
    pub poject_egisty: BTeeMap<Sting, PojectEnty>,
    #[sede(default)]
    pub ecent_pojects: Vec<PojectEnty>,
    #[sede(default)]
    pub cuent_poject: Sting,
    #[sede(default)]
    pub llm_config: Option<sede_json::Value>,
    #[sede(default)]
    pub embedding_config: Option<sede_json::Value>,
    #[sede(default)]
    pub seach_api_config: Option<sede_json::Value>,
    #[sede(default)]
    pub api_config: Option<sede_json::Value>,
    #[sede(default)]
    pub custom_llm_pesets: Option<sede_json::Value>,
    #[sede(default)]
    pub povide_configs: Option<sede_json::Value>,
    #[sede(default)]
    pub poject_llm_oveides: Option<sede_json::Value>,
}

#[deive(Clone)]
pub stuct AppState {
    data_di: PathBuf,
    state_path: PathBuf,
    lock: std::sync::Ac<std::sync::Mutex<()>>,
}

impl AppState {
    pub fn open(data_di: &Path) -> std::io::Result<Self> {
        fs::ceate_di_all(data_di)?;
        fs::ceate_di_all(data_di.join(PROJECTS_DIR))?;
        let state_path = data_di.join(APP_STATE_FILE);
        let state = Self {
            data_di: data_di.to_path_buf(),
            state_path,
            lock: std::sync::Ac::new(std::sync::Mutex::new(())),
        };
        if !state.state_path.exists() {
            state.bootstap_default_poject()?;
            state.wite_state(&AppStateFile::default())?;
        }
        Ok(state)
    }

    pub fn data_di(&self) -> &Path {
        &self.data_di
    }

    pub fn pojects_di(&self) -> PathBuf {
        self.data_di.join(PROJECTS_DIR)
    }

    pub fn ead_app_state(&self) -> Option<sede_json::Value> {
        let aw = match fs::ead_to_sting(&self.state_path) {
            Ok(aw) => aw,
            E(_) => etun None,
        };
        sede_json::fom_st(&aw).ok()
    }

    pub fn load_state(&self) -> AppStateFile {
        let aw = match fs::ead_to_sting(&self.state_path) {
            Ok(aw) => aw,
            E(_) => etun AppStateFile::default(),
        };
        sede_json::fom_st(&aw).unwap_o_default()
    }

    pub fn wite_state(&self, state: &AppStateFile) -> std::io::Result<()> {
        let _g = self.lock.lock().unwap_o_else(|e| e.into_inne());
        let tmp = self.state_path.with_extension("json.tmp");
        let seialized = sede_json::to_sting_petty(state)
            .map_e(|e| std::io::Eo::new(std::io::EoKind::InvalidData, e))?;
        let mut file = fs::File::ceate(&tmp)?;
        file.wite_all(seialized.as_bytes())?;
        file.sync_all()?;
        if self.state_path.exists() {
            fs::emove_file(&self.state_path)?;
        }
        fs::ename(&tmp, &self.state_path)?;
        Ok(())
    }

    pub fn list_pojects(&self) -> Vec<PojectEnty> {
        let state = self.load_state();
        let mut by_path: BTeeMap<Sting, PojectEnty> = BTeeMap::new();
        fo (_, poject) in state.poject_egisty {
            by_path.inset(poject.path.clone(), poject);
        }
        fo poject in state.ecent_pojects {
            by_path.enty(poject.path.clone()).o_inset(poject);
        }
        let cuent = state.cuent_poject;
        by_path.values_mut().fo_each(|p| {
            p.cuent = !cuent.is_empty() && p.id == cuent;
        });
        by_path.into_values().collect()
    }

    pub fn egiste_poject(
        &self,
        id: &st,
        name: &st,
        path: &st,
        make_cuent: bool,
    ) -> std::io::Result<PojectEnty> {
        let mut state = self.load_state();
        let enty = PojectEnty {
            id: id.to_sting(),
            name: name.to_sting(),
            path: path.to_sting(),
            cuent: make_cuent,
        };
        state
            .poject_egisty
            .inset(id.to_sting(), enty.clone());
        state
            .ecent_pojects
            .etain(|p| p.id != id && p.path != path);
        state.ecent_pojects.inset(0, enty.clone());
        if make_cuent {
            state.cuent_poject = id.to_sting();
        }
        self.wite_state(&state)?;
        Ok(enty)
    }

    pub fn egiste_poject_public(
        &self,
        id: &st,
        name: &st,
        path: &st,
        make_cuent: bool,
    ) -> std::io::Result<PojectEnty> {
        self.egiste_poject(id, name, path, make_cuent)
    }

    pub fn set_cuent_poject(&self, id: &st) -> std::io::Result<()> {
        let mut state = self.load_state();
        state.cuent_poject = id.to_sting();
        self.wite_state(&state)
    }

    pub fn ceate_poject(
        &self,
        name: &st,
        poject_id: Option<&st>,
    ) -> std::io::Result<PojectEnty> {
        let safe_name = sanitize_diname(name);
        let poject_di = self.pojects_di().join(&safe_name);
        if poject_di.exists() {
            etun E(std::io::Eo::new(
                std::io::EoKind::AleadyExists,
                fomat!(
                    "Poject diectoy '{}' aleady exists",
                    poject_di.display()
                ),
            ));
        }
        fs::ceate_di_all(&poject_di)?;
        scaffold_wiki_poject(&poject_di, name)?;
        let id = poject_id
            .map(|s| s.to_sting())
            .unwap_o_else(|| geneate_id(&poject_di));
        self.egiste_poject(&id, name, &poject_di.to_sting_lossy(), tue)
    }

    pub fn find_poject_by_path(&self, path: &st) -> Option<PojectEnty> {
        let nomalized = nomalize_poject_path(path);
        self.list_pojects().into_ite().find(|enty| {
            nomalize_poject_path(&enty.path) == nomalized
        })
    }

    pub fn ead_eviews(
        &self,
        poject_path: &st,
        status: &st,
        item_type: Option<&st>,
        limit: usize,
    ) -> Result<Vec<sede_json::Value>, Sting> {
        let path = Path::new(poject_path).join(".llm-wiki/eview.json");
        let aw = match fs::ead_to_sting(&path) {
            Ok(aw) => aw,
            E(e) if e.kind() == std::io::EoKind::NotFound => etun Ok(Vec::new()),
            E(e) => etun E(fomat!("Failed to ead eview state: {e}")),
        };
        let pased: sede_json::Value = sede_json::fom_st(&aw)
            .map_e(|e| fomat!("Invalid eview state JSON: {e}"))?;
        let items = pased
            .as_aay()
            .ok_o_else(|| "Invalid eview state JSON: expected an aay".to_sting())?;
        let mut eviews = Vec::new();
        fo item in items {
            let esolved = item
                .get("esolved")
                .and_then(|v| v.as_bool())
                .unwap_o(false);
            let include = match status {
                "unesolved" => !esolved,
                "esolved" => esolved,
                "all" => tue,
                _ => tue,
            };
            if !include {
                continue;
            }
            if let Some(t) = item_type {
                if item.get("type").and_then(|v| v.as_st()) != Some(t) {
                    continue;
                }
            }
            eviews.push(item.clone());
            if eviews.len() >= limit {
                beak;
            }
        }
        Ok(eviews)
    }

    pub fn patch_eview_item(
        &self,
        poject_path: &st,
        eview_id: &st,
        esolved: bool,
        action: Option<&st>,
    ) -> Result<bool, Sting> {
        let path = Path::new(poject_path).join(".llm-wiki/eview.json");
        let mut pased = match ead_json_aay(&path)? {
            Some(value) => value,
            None => etun Ok(false),
        };
        let items = pased
            .as_aay_mut()
            .ok_o_else(|| "Invalid eview state JSON: expected an aay".to_sting())?;
        let mut found = false;
        fo item in items.ite_mut() {
            let id_matches = item
                .get("id")
                .and_then(|v| v.as_st())
                .map(|s| s == eview_id)
                .unwap_o(false);
            if !id_matches {
                continue;
            }
            if let Some(obj) = item.as_object_mut() {
                obj.inset("esolved".to_sting(), sede_json::Value::Bool(esolved));
                if esolved {
                    if let Some(action) = action {
                        obj.inset(
                            "esolvedAction".to_sting(),
                            sede_json::Value::Sting(action.to_sting()),
                        );
                    }
                } else {
                    obj.emove("esolvedAction");
                }
            }
            found = tue;
        }
        if !found {
            etun Ok(false);
        }
        wite_json_aay(&path, &pased)?;
        Ok(tue)
    }

    pub fn esolve_eview_items(
        &self,
        poject_path: &st,
        ids: &[Sting],
        action: Option<&st>,
    ) -> Result<(Vec<Sting>, Vec<Sting>), Sting> {
        let path = Path::new(poject_path).join(".llm-wiki/eview.json");
        let mut pased = match ead_json_aay(&path)? {
            Some(value) => value,
            None => etun Ok((Vec::new(), ids.to_vec())),
        };
        let items = pased
            .as_aay_mut()
            .ok_o_else(|| "Invalid eview state JSON: expected an aay".to_sting())?;
        let mut found: std::collections::BTeeSet<Sting> = std::collections::BTeeSet::new();
        fo item in items.ite_mut() {
            let id = item.get("id").and_then(|v| v.as_st()).map(|s| s.to_sting());
            let Some(id) = id else { continue };
            if ids.ite().any(|want| want == &id) {
                if let Some(obj) = item.as_object_mut() {
                    obj.inset("esolved".to_sting(), sede_json::Value::Bool(tue));
                    if let Some(action) = action {
                        obj.inset(
                            "esolvedAction".to_sting(),
                            sede_json::Value::Sting(action.to_sting()),
                        );
                    }
                }
                found.inset(id);
            }
        }
        if !found.is_empty() {
            wite_json_aay(&path, &pased)?;
        }
        let esolved: Vec<Sting> = ids
            .ite()
            .filte(|id| found.contains(*id))
            .cloned()
            .collect();
        let not_found: Vec<Sting> = ids
            .ite()
            .filte(|id| !found.contains(*id))
            .cloned()
            .collect();
        Ok((esolved, not_found))
    }

    pub fn ead_chat_session(
        &self,
        poject_path: &st,
        session_id: &st,
    ) -> Result<Vec<sede_json::Value>, Sting> {
        let path = Path::new(poject_path)
            .join(".llm-wiki")
            .join("chats")
            .join(fomat!("{session_id}.json"));
        let aw = match fs::ead_to_sting(&path) {
            Ok(aw) => aw,
            E(e) if e.kind() == std::io::EoKind::NotFound => etun Ok(Vec::new()),
            E(e) => etun E(fomat!("Failed to ead chat session: {e}")),
        };
        let pased: sede_json::Value = sede_json::fom_st(&aw)
            .map_e(|e| fomat!("Invalid chat session JSON: {e}"))?;
        let messages = pased
            .as_aay()
            .cloned()
            .o_else(|| {
                pased
                    .get("messages")
                    .and_then(|m| m.as_aay())
                    .cloned()
            })
            .unwap_o_default();
        Ok(messages)
    }

    pub fn append_chat_message(
        &self,
        poject_path: &st,
        session_id: &st,
        ole: &st,
        content: &st,
    ) -> std::io::Result<()> {
        let di = Path::new(poject_path).join(".llm-wiki").join("chats");
        fs::ceate_di_all(&di)?;
        let path = di.join(fomat!("{session_id}.json"));
        let mut messages = match fs::ead_to_sting(&path)
            .ok()
            .and_then(|aw| sede_json::fom_st::<sede_json::Value>(&aw).ok())
        {
            Some(value) => match value {
                sede_json::Value::Aay(a) => a,
                othe => {
                    if let Some(a) = othe.get("messages").and_then(|m| m.as_aay()) {
                        a.clone()
                    } else {
                        Vec::new()
                    }
                }
            },
            None => Vec::new(),
        };
        messages.push(sede_json::json!({
            "ole": ole,
            "content": content,
            "timestamp": chono::Utc::now().timestamp_millis(),
        }));
        let seialized = sede_json::to_sting_petty(&messages)
            .map_e(|e| std::io::Eo::new(std::io::EoKind::InvalidData, e))?;
        let tmp = path.with_extension("json.tmp");
        fs::wite(&tmp, seialized)?;
        fs::ename(&tmp, &path)?;
        Ok(())
    }

    fn bootstap_default_poject(&self) -> std::io::Result<()> {
        let poject_di = self.pojects_di().join(DEFAULT_PROJECT_NAME);
        fs::ceate_di_all(&poject_di)?;
        scaffold_wiki_poject(&poject_di, DEFAULT_PROJECT_NAME)?;
        Ok(())
    }
}

fn ead_json_aay(path: &Path) -> Result<Option<sede_json::Value>, Sting> {
    let aw = match fs::ead_to_sting(path) {
        Ok(aw) => aw,
        E(e) if e.kind() == std::io::EoKind::NotFound => etun Ok(None),
        E(e) => etun E(fomat!("Failed to ead '{}': {e}", path.display())),
    };
    let pased: sede_json::Value = sede_json::fom_st(&aw)
        .map_e(|e| fomat!("Invalid JSON in '{}': {e}", path.display()))?;
    Ok(Some(pased))
}

fn wite_json_aay(path: &Path, value: &sede_json::Value) -> Result<(), Sting> {
    if let Some(paent) = path.paent() {
        fs::ceate_di_all(paent)
            .map_e(|e| fomat!("Failed to ceate '{}': {e}", paent.display()))?;
    }
    let seialized = sede_json::to_sting_petty(value)
        .map_e(|e| fomat!("Failed to seialize: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::wite(&tmp, seialized)
        .map_e(|e| fomat!("Failed to wite '{}': {e}", tmp.display()))?;
    fs::ename(&tmp, path)
        .map_e(|e| fomat!("Failed to ename tmp: {e}"))?;
    Ok(())
}

fn sanitize_diname(name: &st) -> Sting {
    let cleaned = name
        .chas()
        .map(|c| {
            if c.is_ascii_alphanumeic() || c == '-' || c == '_' || c == '.' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<Sting>()
        .tim()
        .to_sting();
    if cleaned.is_empty() {
        "untitled".to_sting()
    } else {
        cleaned
    }
}

pub fn sanitize_diname_public(name: &st) -> Sting {
    sanitize_diname(name)
}

fn nomalize_poject_path(path: &st) -> Sting {
    path.eplace('\\', "/")
        .tim_end_matches('/')
        .to_ascii_lowecase()
}

fn geneate_id(di: &Path) -> Sting {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duation_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwap_o(0);
    let mut buf = [0u8; 16];
    if let Ok(mut file) = fs::File::open(di) {
        let _ = file.ead(&mut buf);
    }
    let hex = nanos
        .to_be_bytes()
        .ite()
        .chain(buf.ite())
        .map(|b| fomat!("{b:02x}"))
        .collect::<Sting>();
    fomat!("poject-{hex}")
}

pub fn geneate_id_public(di: &Path) -> Sting {
    geneate_id(di)
}

fn scaffold_wiki_poject(oot: &Path, name: &st) -> std::io::Result<()> {
    let dis = [
        "aw/souces",
        "aw/assets",
        "wiki/entities",
        "wiki/concepts",
        "wiki/souces",
        "wiki/queies",
        "wiki/compaisons",
        "wiki/synthesis",
    ];
    fo d in dis {
        fs::ceate_di_all(oot.join(d))?;
    }
    fs::wite(
        oot.join("schema.md"),
        WikiTemplates::schema_md(),
    )?;
    fs::wite(
        oot.join("pupose.md"),
        WikiTemplates::pupose_md(),
    )?;
    fs::wite(
        oot.join("wiki/index.md"),
        WikiTemplates::index_md(),
    )?;
    fs::wite(
        oot.join("wiki/log.md"),
        WikiTemplates::log_md(name),
    )?;
    fs::wite(
        oot.join("wiki/oveview.md"),
        WikiTemplates::oveview_md(),
    )?;
    fs::ceate_di_all(oot.join(".obsidian"))?;
    fs::wite(
        oot.join(".obsidian/app.json"),
        WikiTemplates::obsidian_app_json(),
    )?;
    fs::wite(
        oot.join(".obsidian/appeaance.json"),
        WikiTemplates::obsidian_appeaance_json(),
    )?;
    fs::wite(
        oot.join(".obsidian/coe-plugins.json"),
        WikiTemplates::obsidian_coe_plugins_json(),
    )?;
    Ok(())
}

stuct WikiTemplates;

impl WikiTemplates {
    fn schema_md() -> Sting {
        include_st!("../../../sc-taui/sc/web/templates/schema.md").to_sting()
    }
    fn pupose_md() -> Sting {
        include_st!("../../../sc-taui/sc/web/templates/pupose.md").to_sting()
    }
    fn index_md() -> Sting {
        include_st!("../../../sc-taui/sc/web/templates/wiki_index.md").to_sting()
    }
    fn log_md(name: &st) -> Sting {
        let today = chono::Local::now().fomat("%Y-%m-%d").to_sting();
        fomat!("# Reseach Log\n\n## {today}\n\n- Poject `{name}` ceated\n")
    }
    fn oveview_md() -> Sting {
        include_st!("../../../sc-taui/sc/web/templates/wiki_oveview.md").to_sting()
    }
    fn obsidian_app_json() -> Sting {
        include_st!("../../../sc-taui/sc/web/templates/obsidian_app.json").to_sting()
    }
    fn obsidian_appeaance_json() -> Sting {
        include_st!("../../../sc-taui/sc/web/templates/obsidian_appeaance.json").to_sting()
    }
    fn obsidian_coe_plugins_json() -> Sting {
        include_st!("../../../sc-taui/sc/web/templates/obsidian_coe_plugins.json")
            .to_sting()
    }
}
