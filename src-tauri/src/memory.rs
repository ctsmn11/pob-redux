//! One local store for user-approved Markdown shared by every assistant transport.
use std::{fs, io::Write, path::{Component, Path, PathBuf}, sync::Mutex};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use crate::{game::Game, AppState};

pub const LIMIT: usize = 16_000;
pub const INSTRUCTIONS: &str = "Read memory_read at the start of work and after changing builds. Memory contains user-approved preferences, durable goals and decisions, not live game facts. Current user requests take precedence. Get statistics and game data from PoB tools. Use memory_propose with an opaque document id and its exact revision to suggest replacement Markdown. Only the user can approve in the app; generic tool approval never commits memory. Never claim a proposal was saved until memory_proposal_status reports approved. Disabled memory must not be used; older notes may still exist in conversation history.";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub revision: u64,
    pub game: Option<Game>,
    pub path: Option<PathBuf>,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub document_id: String,
    pub revision: u64,
    pub author: String,
    pub reason: String,
    pub before: String,
    pub text: String,
    pub status: String,
    pub destination: Document,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Store {
    format_version: u32,
    enabled: bool,
    documents: Vec<Document>,
    proposals: Vec<Proposal>,
}
impl Default for Store {
    fn default() -> Self {
        Self { format_version: 1, enabled: false, documents: vec![], proposals: vec![] }
    }
}
pub struct Memory {
    path: PathBuf,
    gate: Mutex<()>,
}
fn id() -> Result<String, String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn validate(text: &str) -> Result<(), String> {
    if text.chars().count() > LIMIT { Err(format!("memory exceeds {LIMIT} characters")) } else { Ok(()) }
}
pub fn normalize(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() { return Err("memory requires an absolute build path".into()); }
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {},
            Component::ParentDir => { clean.pop(); },
            other => clean.push(other.as_os_str()),
        }
    }
    #[cfg(windows)]
    { clean = PathBuf::from(clean.to_string_lossy().to_lowercase()); }
    Ok(clean)
}
pub fn remap(path: &Path, src: &Path, dst: &Path) -> Option<PathBuf> {
    let path = normalize(path).ok()?;
    let src = normalize(src).ok()?;
    let dst = normalize(dst).ok()?;
    let rest = path.strip_prefix(src).ok()?;
    Some(if rest.as_os_str().is_empty() { dst } else { dst.join(rest) })
}

impl Memory {
    pub fn new(path: PathBuf) -> Self { Self { path, gate: Mutex::new(()) } }
    fn load(&self) -> Result<Store, String> {
        let bytes = match fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
            Err(e) => return Err(format!("cannot read assistant memory: {e}")),
        };
        let store: Store = serde_json::from_slice(&bytes).map_err(|e| format!("cannot read assistant memory; original file preserved: {e}"))?;
        if store.format_version != 1 { return Err("unsupported assistant memory format; original file preserved".into()); }
        let mut ids = std::collections::HashSet::new();
        let mut destinations = std::collections::HashSet::new();
        for d in &store.documents {
            validate(&d.text)?;
            if d.id.is_empty() || !ids.insert(&d.id) || d.game.is_some() != d.path.is_some() {
                return Err("invalid memory document identity; original file preserved".into());
            }
            let key = (d.game.map(Game::id), d.path.clone());
            if !destinations.insert(key) || d.path.as_ref().is_some_and(|p| normalize(p).as_ref() != Ok(p)) {
                return Err("invalid memory destination; original file preserved".into());
            }
        }
        Ok(store)
    }
    fn save(&self, store: &Store) -> Result<(), String> {
        let parent = self.path.parent().ok_or("missing memory directory")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temp = parent.join(format!(".memory-{}.tmp", id()?));
        let result = (|| -> Result<(), String> {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            { use std::os::unix::fs::OpenOptionsExt; opts.mode(0o600); }
            let mut file = opts.open(&temp).map_err(|e| e.to_string())?;
            file.write_all(&serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&temp, &self.path).map_err(|e| e.to_string())?;
            Ok(())
        })();
        if result.is_err() { let _ = fs::remove_file(temp); }
        result
    }
    fn transaction<T>(&self, f: impl FnOnce(&mut Store) -> Result<T, String>) -> Result<T, String> {
        let _guard = self.gate.lock().map_err(|e| e.to_string())?;
        let mut store = self.load()?;
        let before = store.clone();
        let out = f(&mut store)?;
        if store != before { self.save(&store)?; }
        Ok(out)
    }
    fn ensure(store: &mut Store, game: Option<Game>, path: Option<PathBuf>) -> Result<Document, String> {
        if let Some(d) = store.documents.iter().find(|d| d.game == game && d.path == path) { return Ok(d.clone()); }
        let doc = Document { id: id()?, revision: 0, game, path, text: String::new() };
        store.documents.push(doc.clone());
        Ok(doc)
    }
    pub fn read(&self, game: Game, path: Option<&Path>, user: bool) -> Result<Value, String> {
        self.transaction(|s| {
            if !user && !s.enabled { return Ok(json!({ "enabled": false, "game": game, "build": path, "global": null, "current": null })); }
            let global = Self::ensure(s, None, None)?;
            let current = path.map(|p| normalize(p).and_then(|p| Self::ensure(s, Some(game), Some(p)))).transpose()?;
            Ok(if user {
                json!({ "enabled": s.enabled, "game": game, "build": path, "global": global, "current": current,
                    "documents": s.documents.iter().map(|d| json!({ "document": d, "orphan": d.path.as_ref().is_some_and(|p| !p.is_file()) })).collect::<Vec<_>>(), "proposals": s.proposals })
            } else { json!({ "enabled": true, "game": game, "build": path, "global": global, "current": current }) })
        })
    }
    pub fn pending(&self) -> Result<usize, String> {
        let _guard = self.gate.lock().map_err(|e| e.to_string())?;
        Ok(self.load()?.proposals.iter().filter(|p| p.status == "pending").count())
    }
    pub fn enable(&self, enabled: bool) -> Result<(), String> {
        self.transaction(|s| {
            s.enabled = enabled;
            if !enabled { for p in &mut s.proposals { if p.status == "pending" { p.status = "invalidated".into(); } } }
            Ok(())
        })
    }
    pub fn edit(&self, document: &str, revision: u64, text: &str) -> Result<(), String> {
        validate(text)?;
        self.transaction(|s| {
            let d = s.documents.iter_mut().find(|d| d.id == document).ok_or("unknown memory document")?;
            if d.revision != revision { return Err("memory changed; reload before saving".into()); }
            d.text = text.into();
            d.revision = d.revision.checked_add(1).ok_or("memory revision limit reached")?;
            Self::invalidate(s, document);
            Ok(())
        })
    }
    fn invalidate(s: &mut Store, document: &str) {
        for p in &mut s.proposals {
            if p.document_id == document && p.status == "pending" { p.status = "invalidated".into(); }
        }
    }
    pub fn delete(&self, document: &str, revision: u64) -> Result<(), String> {
        self.transaction(|s| {
            let d = s.documents.iter_mut().find(|d| d.id == document).ok_or("unknown memory document")?;
            if d.revision != revision { return Err("memory changed; reload before deleting".into()); }
            Self::invalidate(s, document);
            s.documents.retain(|d| d.id != document);
            Ok(())
        })
    }
    pub fn propose(&self, game: Game, path: Option<&Path>, document: &str, revision: u64, text: &str, reason: &str, author: &str) -> Result<Proposal, String> {
        validate(text)?;
        if reason.trim().is_empty() || reason.chars().count() > 2000 { return Err("reason must contain 1 to 2000 characters".into()); }
        let path = path.map(normalize).transpose()?;
        self.transaction(|s| {
            if !s.enabled { return Err("assistant memory is disabled".into()); }
            let d = s.documents.iter().find(|d| d.id == document).ok_or("unknown memory document; call memory_read")?;
            if d.game.is_some() && (d.game != Some(game) || d.path != path || path.is_none()) { return Err("memory document is not the current saved build".into()); }
            if d.revision != revision { return Err("memory changed; read it and submit a new proposal".into()); }
            let p = Proposal { id: id()?, document_id: d.id.clone(), revision, author: author.into(), reason: reason.into(), before: d.text.clone(), text: text.into(), status: "pending".into(), destination: d.clone() };
            s.proposals.push(p.clone());
            Ok(p)
        })
    }
    pub fn status(&self, proposal: &str) -> Result<Value, String> {
        let _guard = self.gate.lock().map_err(|e| e.to_string())?;
        let s = self.load()?;
        let p = s.proposals.iter().find(|p| p.id == proposal).ok_or("unknown memory proposal")?;
        Ok(json!({ "id": p.id, "document_id": p.document_id, "status": p.status }))
    }
    pub fn resolve(&self, proposal: &str, approve: bool) -> Result<(), String> {
        self.transaction(|s| {
            let index = s.proposals.iter().position(|p| p.id == proposal).ok_or("unknown memory proposal")?;
            let p = s.proposals[index].clone();
            if p.status != "pending" { return Err("proposal is no longer pending; request a new proposal".into()); }
            if !approve { s.proposals[index].status = "rejected".into(); return Ok(()); }
            if !s.enabled { return Err("assistant memory is disabled".into()); }
            let d = s.documents.iter_mut().find(|d| d.id == p.document_id).ok_or("memory document no longer exists")?;
            if d.revision != p.revision || d.game != p.destination.game || d.path != p.destination.path {
                return Err("proposal is obsolete; request a new proposal".into());
            }
            validate(&p.text)?;
            d.text = p.text;
            d.revision = d.revision.checked_add(1).ok_or("memory revision limit reached")?;
            Self::invalidate(s, &p.document_id);
            s.proposals[index].status = "approved".into();
            Ok(())
        })
    }
    pub fn relocate(&self, game: Game, src: &Path, dst: Option<&Path>, copy: bool) -> Result<(), String> {
        let src = normalize(src)?;
        let dst = dst.map(normalize).transpose()?;
        self.transaction(|s| {
            let docs = s.documents.clone();
            for d in docs.iter().filter(|d| d.game == Some(game) && d.path.as_ref().is_some_and(|p| p.starts_with(&src))) {
                let target = dst.as_ref().and_then(|dst| remap(d.path.as_ref().unwrap(), &src, dst));
                if let Some(target) = target {
                    let occupied = s.documents.iter().any(|other| other.id != d.id && other.game == Some(game) && other.path.as_ref() == Some(&target));
                    if copy {
                        if !occupied && d.path.as_ref() != Some(&target) {
                            s.documents.push(Document { id: id()?, revision: 0, game: d.game, path: Some(target), text: d.text.clone() });
                        }
                    } else {
                        Self::invalidate(s, &d.id);
                        if occupied { s.documents.retain(|other| other.id != d.id); }
                        else if let Some(doc) = s.documents.iter_mut().find(|other| other.id == d.id) { doc.path = Some(target); }
                    }
                } else {
                    Self::invalidate(s, &d.id);
                    s.documents.retain(|other| other.id != d.id);
                }
            }
            Ok(())
        })
    }
}

pub fn identity(state: &AppState) -> (Game, Option<PathBuf>) {
    let rt = state.rt.read().unwrap();
    let path = rt.engine.call("get_build", Value::Null).ok()
        .and_then(|b| b.result.get("file").and_then(Value::as_str).map(PathBuf::from))
        .filter(|p| p.is_absolute() && p.is_file());
    (rt.game, path)
}
fn prompt(memory: &Memory, game: Game, path: Option<&Path>) -> String {
    match memory.read(game, path, false) {
        Ok(data) => format!("{}\nCurrent assistant memory (refreshed for this message):\n{}", INSTRUCTIONS, data),
        Err(_) => format!("{}\nAssistant memory is unavailable; do not infer or reuse stored notes. Current game: {}; saved build: {:?}", INSTRUCTIONS, game.id(), path),
    }
}
pub fn context(app: &AppHandle) -> String {
    let state = app.state::<AppState>();
    let _guard = state.engine_gate.lock().unwrap();
    let (game, path) = identity(&state);
    prompt(&state.memory, game, path.as_deref())
}
pub fn changed(app: &AppHandle) { let _ = app.emit("memory:changed", ()); }

#[tauri::command]
pub fn memory_context(app: AppHandle) -> String { context(&app) }
#[tauri::command]
pub fn memory_pending(state: State<'_, AppState>) -> Result<usize, String> { state.memory.pending() }
#[tauri::command]
pub fn memory_manage(app: AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let _guard = state.engine_gate.lock().unwrap();
    let (game, path) = identity(&state);
    state.memory.read(game, path.as_deref(), true)
}
#[tauri::command]
pub fn memory_enable(app: AppHandle, state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    state.memory.enable(enabled)?; changed(&app); Ok(())
}
#[tauri::command]
pub fn memory_edit(app: AppHandle, state: State<'_, AppState>, document: String, revision: u64, text: String) -> Result<(), String> {
    state.memory.edit(&document, revision, &text)?; changed(&app); Ok(())
}
#[tauri::command]
pub fn memory_delete(app: AppHandle, state: State<'_, AppState>, document: String, revision: u64) -> Result<(), String> {
    state.memory.delete(&document, revision)?; changed(&app); Ok(())
}
#[tauri::command]
pub fn memory_resolve(app: AppHandle, state: State<'_, AppState>, proposal: String, approve: bool) -> Result<(), String> {
    state.memory.resolve(&proposal, approve)?; changed(&app); Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture { root: PathBuf, memory: Memory, a: PathBuf, b: PathBuf }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("assistant-memory-{}", id().unwrap()));
            fs::create_dir_all(&root).unwrap();
            let a = root.join("a.xml"); let b = root.join("b.xml");
            fs::write(&a, "build a").unwrap(); fs::write(&b, "build b").unwrap();
            Self { memory: Memory::new(root.join("memory/store.json")), root, a, b }
        }
        fn read(&self, path: Option<&Path>, user: bool) -> Value { self.memory.read(Game::Poe2, path, user).unwrap() }
        fn doc(&self, path: Option<&Path>) -> Document {
            let v = self.read(path, true);
            serde_json::from_value(v[if path.is_some() { "current" } else { "global" }].clone()).unwrap()
        }
        fn propose(&self, d: &Document, text: &str) -> Proposal {
            self.memory.propose(Game::Poe2, d.path.as_deref(), &d.id, d.revision, text, "durable preference", "test assistant").unwrap()
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); } }

    #[test]
    fn default_off_persists_and_user_can_edit_while_disabled() {
        let f = Fixture::new();
        assert_eq!(f.read(Some(&f.a), false)["enabled"], false);
        assert!(f.read(Some(&f.a), false)["global"].is_null());
        let d = f.doc(None);
        f.memory.edit(&d.id, d.revision, "# Preferences\nFewer buttons").unwrap();
        assert!(f.memory.propose(Game::Poe2, None, &d.id, 1, "x", "why", "agent").is_err());
        f.memory.enable(true).unwrap();
        let reopened = Memory::new(f.memory.path.clone());
        let v = reopened.read(Game::Poe1, None, false).unwrap();
        assert_eq!(v["enabled"], true);
        assert_eq!(v["global"]["text"], "# Preferences\nFewer buttons");
        assert!(v["current"].is_null());
    }
    #[test]
    fn proposals_never_commit_without_app_resolution_and_survive_restart() {
        let f = Fixture::new(); f.memory.enable(true).unwrap();
        let d = f.doc(Some(&f.a)); let p = f.propose(&d, "Use a shield");
        assert_eq!(f.doc(Some(&f.a)).text, "");
        let reopened = Memory::new(f.memory.path.clone());
        assert_eq!(reopened.status(&p.id).unwrap()["status"], "pending");
        // A different build can be open when the user approves.
        f.doc(Some(&f.b));
        reopened.resolve(&p.id, true).unwrap();
        assert_eq!(f.doc(Some(&f.a)).text, "Use a shield");
        assert_eq!(f.doc(Some(&f.b)).text, "");
        assert_eq!(reopened.status(&p.id).unwrap()["status"], "approved");
        assert!(reopened.resolve(&p.id, true).is_err());
    }
    #[test]
    fn concurrent_proposals_and_manual_edits_cannot_overwrite_newer_notes() {
        let f = Fixture::new(); f.memory.enable(true).unwrap();
        let d = f.doc(None); let a = f.propose(&d, "first"); let b = f.propose(&d, "second");
        f.memory.resolve(&a.id, true).unwrap();
        assert!(f.memory.resolve(&b.id, true).is_err());
        assert_eq!(f.memory.status(&b.id).unwrap()["status"], "invalidated");
        let d = f.doc(None); let c = f.propose(&d, "third");
        f.memory.edit(&d.id, d.revision, "manual").unwrap();
        assert!(f.memory.resolve(&c.id, true).is_err());
        assert!(f.memory.edit(&d.id, d.revision, "stale").is_err());
        assert_eq!(f.doc(None).text, "manual");
    }
    #[test]
    fn disable_invalidates_pending_and_blocks_reads_and_proposals() {
        let f = Fixture::new(); f.memory.enable(true).unwrap();
        let d = f.doc(None); f.memory.edit(&d.id, 0, "private").unwrap();
        let d = f.doc(None); let p = f.propose(&d, "next");
        f.memory.enable(false).unwrap();
        assert!(f.read(None, false)["global"].is_null());
        assert!(f.memory.resolve(&p.id, true).is_err());
        assert!(f.memory.propose(Game::Poe2, None, &d.id, 1, "x", "why", "agent").is_err());
        assert_eq!(f.doc(None).text, "private");
        f.memory.enable(true).unwrap();
        assert_eq!(f.memory.status(&p.id).unwrap()["status"], "invalidated");
    }
    #[test]
    fn isolation_opaque_targets_unsaved_builds_and_rejection() {
        let f = Fixture::new(); f.memory.enable(true).unwrap();
        let a = f.doc(Some(&f.a)); let b = f.doc(Some(&f.b));
        let poe1 = f.memory.read(Game::Poe1, Some(&f.a), true).unwrap();
        assert_ne!(a.id, b.id); assert_ne!(a.id, poe1["current"]["id"]);
        assert_eq!(f.read(None, true)["global"]["id"], poe1["global"]["id"]);
        assert!(f.read(None, false)["current"].is_null());
        for (game, path, target) in [(Game::Poe1, Some(f.a.as_path()), a.id.as_str()), (Game::Poe2, Some(f.b.as_path()), a.id.as_str()), (Game::Poe2, None, a.id.as_str()), (Game::Poe2, Some(f.a.as_path()), "/arbitrary/path")] {
            assert!(f.memory.propose(game, path, target, 0, "x", "why", "agent").is_err());
        }
        let p = f.propose(&a, "rejected"); f.memory.resolve(&p.id, false).unwrap();
        assert_eq!(f.memory.status(&p.id).unwrap()["status"], "rejected");
        assert_eq!(f.doc(Some(&f.a)).text, "");
    }
    #[test]
    fn unicode_limit_is_characters_and_never_truncates() {
        let f = Fixture::new(); f.memory.enable(true).unwrap(); let d = f.doc(None);
        let exact = "🛡".repeat(LIMIT); let too_long = format!("{exact}x");
        f.memory.edit(&d.id, 0, &exact).unwrap();
        assert!(f.memory.edit(&d.id, 1, &too_long).is_err());
        assert!(f.memory.propose(Game::Poe2, None, &d.id, 1, &too_long, "why", "agent").is_err());
        assert_eq!(f.doc(None).text, exact);
    }
    #[test]
    fn corruption_and_write_errors_preserve_original_and_pending_status() {
        let f = Fixture::new(); f.memory.enable(true).unwrap(); let d = f.doc(None); let p = f.propose(&d, "x");
        let original = fs::read(&f.memory.path).unwrap();
        fs::write(&f.memory.path, b"corrupted").unwrap();
        assert!(f.memory.enable(false).is_err()); assert!(f.memory.edit(&d.id, 0, "overwrite").is_err());
        assert_eq!(fs::read(&f.memory.path).unwrap(), b"corrupted");
        fs::write(&f.memory.path, &original).unwrap();
        // A file in place of the directory gives a deterministic write/read error even as root.
        let bad = Memory::new(f.a.join("store.json"));
        assert!(bad.enable(true).is_err());
        assert_eq!(fs::read_to_string(&f.a).unwrap(), "build a");
        assert_eq!(f.memory.status(&p.id).unwrap()["status"], "pending");
    }
    #[test]
    fn rename_folder_move_save_as_existing_destination_and_delete() {
        let f = Fixture::new(); f.memory.enable(true).unwrap();
        let a = f.doc(Some(&f.a)); f.memory.edit(&a.id, 0, "a notes").unwrap();
        let p = f.propose(&f.doc(Some(&f.a)), "pending");
        let dir = f.root.join("folder"); fs::create_dir(&dir).unwrap(); let moved = dir.join("a.xml");
        fs::rename(&f.a, &moved).unwrap();
        f.memory.relocate(Game::Poe2, &f.a, Some(&moved), false).unwrap();
        assert_eq!(f.doc(Some(&moved)).id, a.id);
        assert!(f.doc(Some(&moved)).path.unwrap().is_file());
        assert_eq!(f.memory.status(&p.id).unwrap()["status"], "invalidated");
        let renamed_dir = f.root.join("renamed"); fs::rename(&dir, &renamed_dir).unwrap();
        f.memory.relocate(Game::Poe2, &dir, Some(&renamed_dir), false).unwrap();
        let moved = renamed_dir.join("a.xml"); assert_eq!(f.doc(Some(&moved)).text, "a notes");
        f.memory.relocate(Game::Poe2, &moved, Some(&f.b), true).unwrap();
        let b = f.doc(Some(&f.b)); assert_ne!(b.id, a.id); assert_eq!(b.text, "a notes");
        f.memory.edit(&b.id, 0, "independent").unwrap();
        f.memory.relocate(Game::Poe2, &moved, Some(&f.b), true).unwrap();
        assert_eq!(f.doc(Some(&f.b)).text, "independent"); assert_eq!(f.doc(Some(&moved)).text, "a notes");
        fs::remove_file(&moved).unwrap();
        assert!(f.read(None, true)["documents"].as_array().unwrap().iter().any(|d| d["document"]["id"] == a.id && d["orphan"] == true));
        f.memory.relocate(Game::Poe2, &moved, None, false).unwrap();
        assert!(!f.read(None, true)["documents"].as_array().unwrap().iter().any(|d| d["document"]["id"] == a.id));
    }
    #[test]
    fn serializes_parallel_approvals() {
        let f = Fixture::new(); f.memory.enable(true).unwrap(); let d = f.doc(None);
        let a = f.propose(&d, "a"); let b = f.propose(&d, "b");
        let results = std::thread::scope(|scope| {
            let first = scope.spawn(|| f.memory.resolve(&a.id, true));
            let second = scope.spawn(|| f.memory.resolve(&b.id, true));
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_ne!(results.0.is_ok(), results.1.is_ok());
        assert_eq!(f.doc(None).revision, 1);
    }
    #[test]
    fn normalizes_absolute_paths_and_rejects_relative_paths() {
        let f = Fixture::new();
        let a = f.doc(Some(&f.a));
        assert_eq!(a.id, f.doc(Some(&f.root.join("dir/../a.xml"))).id);
        assert!(normalize(Path::new("../a.xml")).is_err());
    }
    #[test]
    fn refreshed_prompt_tracks_edits_build_switch_game_and_disable() {
        let f = Fixture::new(); f.memory.enable(true).unwrap();
        let global = f.doc(None); f.memory.edit(&global.id, 0, "Global preference").unwrap();
        let a = f.doc(Some(&f.a)); f.memory.edit(&a.id, 0, "Only build A").unwrap();
        let first = prompt(&f.memory, Game::Poe2, Some(&f.a));
        assert!(first.contains("Global preference") && first.contains("Only build A"));
        assert!(first.contains(f.a.to_str().unwrap()));
        let next = prompt(&f.memory, Game::Poe2, Some(&f.b));
        assert!(next.contains("Global preference") && !next.contains("Only build A"));
        assert!(next.contains(f.b.to_str().unwrap()));
        assert!(!prompt(&f.memory, Game::Poe1, Some(&f.a)).contains("Only build A"));
        f.memory.edit(&global.id, 1, "Updated preference").unwrap();
        let next = prompt(&f.memory, Game::Poe2, None);
        assert!(next.contains("Updated preference") && !next.contains("Global preference"));
        f.memory.enable(false).unwrap();
        let off = prompt(&f.memory, Game::Poe2, Some(&f.a));
        assert!(off.contains("\"enabled\":false"));
        assert!(!off.contains("Updated preference") && !off.contains("Only build A"));
    }
    #[test]
    fn failed_atomic_replace_leaves_no_partial_file_or_temp_files() {
        let f = Fixture::new();
        fs::create_dir_all(&f.memory.path).unwrap();
        fs::write(f.memory.path.join("original"), "unchanged").unwrap();
        assert!(f.memory.save(&Store::default()).is_err());
        assert_eq!(fs::read_to_string(f.memory.path.join("original")).unwrap(), "unchanged");
        assert_eq!(fs::read_dir(f.memory.path.parent().unwrap()).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn reads_work_on_readonly_store_and_failed_write_preserves_notes() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new(); f.memory.enable(true).unwrap();
        let d = f.doc(None); f.memory.edit(&d.id, 0, "approved").unwrap();
        let original = fs::read(&f.memory.path).unwrap();
        let dir = f.memory.path.parent().unwrap();
        fs::set_permissions(dir, fs::Permissions::from_mode(0o500)).unwrap();
        let read = f.memory.read(Game::Poe2, None, false);
        let edit = f.memory.edit(&d.id, 1, "lost write");
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(read.unwrap()["global"]["text"], "approved");
        if unsafe { libc::geteuid() } != 0 {
            assert!(edit.is_err());
            assert_eq!(fs::read(&f.memory.path).unwrap(), original);
        }
    }
    #[test]
    fn remapping_a_file_does_not_add_a_directory_separator() {
        let f = Fixture::new();
        let mapped = remap(&f.a, &f.a, &f.b).unwrap();
        assert!(mapped.is_file());
        assert_eq!(mapped.as_os_str(), f.b.as_os_str());
        assert_eq!(remap(&f.a, &f.root, &f.root.join("moved")).unwrap(), f.root.join("moved/a.xml"));
    }
}
