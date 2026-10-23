//! Reviewed trusted metadata uses the existing SQLite writer and quota.
use super::*;
use crate::providers::{
    Agent,
    catalog::{self, Document, Selection},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub project: String,
    pub operation: Operation,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Load,
    Import {
        path: PathBuf,
        agent: Agent,
        connection_scope: String,
    },
    Save {
        document: Document,
        expected: Option<String>,
    },
    Disconnect {
        expected: String,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    Loaded(Option<Document>),
    Imported(Document),
    Saved(Document),
    Disconnected(Option<Document>),
}

pub(super) fn migrate(connection: &Connection) -> Result<(), String> {
    connection.execute_batch("CREATE TABLE tool_catalog(project TEXT PRIMARY KEY,document TEXT NOT NULL CHECK(length(CAST(document AS BLOB))<=524288));").map_err(|e| e.to_string())
}
pub(super) fn inspect(connection: &Connection) -> Result<(), String> {
    let columns = connection
        .prepare("PRAGMA table_info(tool_catalog)")
        .map_err(|e| e.to_string())?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if columns
        != vec![
            ("project".into(), "TEXT".into(), 0, 1),
            ("document".into(), "TEXT".into(), 1, 0),
        ]
    {
        return Err("Unexpected trusted tool catalog columns".into());
    }
    let mut statement = connection
        .prepare("SELECT project,document FROM tool_catalog")
        .map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let project: String = row.get(0).map_err(|e| e.to_string())?;
        if project.is_empty() || project.len() > 4096 {
            return Err("Invalid catalog project identity".into());
        }
        let text: String = row.get(1).map_err(|e| e.to_string())?;
        decode(&text)?;
    }
    Ok(())
}
fn decode(text: &str) -> Result<Document, String> {
    if text.len() > catalog::MAX_BYTES {
        return Err("Stored catalog exceeds 512 KiB".into());
    }
    let document: Document =
        serde_json::from_str(text).map_err(|e| format!("Invalid stored trusted catalog: {e}"))?;
    document.validate_stored()?;
    Ok(document)
}
fn load(connection: &Connection, project: &str) -> Result<Option<Document>, String> {
    let text: Option<String> = connection
        .query_row(
            "SELECT document FROM tool_catalog WHERE project=?1",
            [project],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    text.map(|text| decode(&text)).transpose()
}
pub fn document_digest(document: &Document) -> Result<String, String> {
    Ok(catalog::digest(
        &serde_json::to_vec(document).map_err(|_| "Cannot encode catalog")?,
    ))
}

impl State {
    pub(super) fn tool_catalog_request(&mut self, request: &Request) -> Result<Response, String> {
        let request = request.clone();
        if request.project != self.project {
            return Err("Catalog belongs to another project".into());
        }
        let connection = self.connection.as_mut().ok_or("Storage unavailable")?;
        match request.operation {
            Operation::Load => Ok(Response::Loaded(load(connection, &self.project)?)),
            Operation::Import {
                path,
                agent,
                connection_scope,
            } => {
                let document = Document::import(&path, agent, connection_scope)?;
                Ok(Response::Imported(document))
            }
            Operation::Save { document, expected } => {
                document.validate()?;
                document.revalidate(&document.connection_scope)?;
                let tx = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| e.to_string())?;
                no_active(&tx, None)?;
                let current = load(&tx, &self.project)?;
                if current.as_ref().map(document_digest).transpose()? != expected {
                    return Err("Trusted catalog changed; reload setup review".into());
                }
                let text = serde_json::to_string(&document).map_err(|_| "Cannot encode catalog")?;
                tx.execute("INSERT INTO tool_catalog(project,document) VALUES(?1,?2) ON CONFLICT(project) DO UPDATE SET document=excluded.document", params![self.project, text]).map_err(|e| e.to_string())?;
                enforce_evidence_quota(&tx, &self.data_dir, self.policy)?;
                tx.commit().map_err(|e| e.to_string())?;
                Ok(Response::Saved(document))
            }
            Operation::Disconnect { expected } => {
                let tx = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|e| e.to_string())?;
                let mut document = load(&tx, &self.project)?;
                if let Some(document) = &mut document
                    && document_digest(document)? == expected
                {
                    document.connection_scope.clear();
                    let text = serde_json::to_string(document)
                        .map_err(|_| "Cannot encode disconnected catalog")?;
                    tx.execute(
                        "UPDATE tool_catalog SET document=?2 WHERE project=?1",
                        params![self.project, text],
                    )
                    .map_err(|e| e.to_string())?;
                }
                tx.commit().map_err(|e| e.to_string())?;
                Ok(Response::Disconnected(document))
            }
        }
    }
}

/// No writer, repair or schema discovery; exact persisted setup is checked again
/// before every HTTP round, so revoked/replaced metadata cannot retain authority.
pub fn validate_selection(
    data_dir: &Path,
    project: &str,
    selected: &Selection,
) -> Result<(), String> {
    let path = canonical_parent(&data_dir.join(DATABASE))?;
    let access = store_access(path.parent().ok_or("Catalog database parent missing")?);
    let _guard = access.lock();
    let connection = open_readonly_canonical(&path)?;
    let document = load(&connection, project)?
        .ok_or("Catalog setup is no longer stored; reviewed setup required")?;
    if document_digest(&document)? != selected.document_digest
        || document.connection_scope.is_empty()
    {
        return Err("Stored trusted catalog changed/disconnected; reviewed setup required".into());
    }
    for entry in &selected.entries {
        if !document.manifest.tools.contains(entry) {
            return Err("Selected exact schema is absent from current reviewed setup".into());
        }
    }
    Ok(())
}
