//! Album tree, membership and ordering.
//!
//! Albums mirror the folder structure the web gallery publishes:
//! `2026/weddings/ana-ivan`. Every field maps to gallery frontmatter — see
//! [`crate::publish`] for the contract.

use rand::Rng;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Deserializer, Serialize};

use crate::catalog::{album_from_row, Library, ALBUM_COLS};
use crate::error::{Error, Result};
use crate::model::Album;

/// Fields settable when creating an album. Everything else takes a default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAlbum {
    pub path: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub is_collection: bool,
}

/// Distinguish "field absent" from "field explicitly null" when deserializing.
///
/// Plain `Option<Option<T>>` collapses `null` to the outer `None`, which would
/// make clearing a field impossible over JSON. With this, the UI can send
/// `{"password": null}` to clear and omit the key to leave it alone.
fn double_option<'de, T, D>(de: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

/// Partial update. `None` leaves a field untouched; `Some(None)` clears it.
/// This mirrors the merge semantics the web admin uses, so a partial form can
/// never wipe a field it doesn't know about.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumUpdate {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub date: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub password: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub share_token: Option<Option<String>>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub cover_photo_id: Option<Option<i64>>,
    #[serde(default)]
    pub is_collection: Option<bool>,
    #[serde(default)]
    pub hidden: Option<bool>,
    #[serde(default)]
    pub allow_download: Option<bool>,
    #[serde(default)]
    pub proofing: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub sort_order: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    pub body: Option<Option<String>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Normalise a user-supplied album path: lowercase, no leading/trailing
/// slashes, no traversal. Matches the web side's folder rules.
pub fn normalize_path(input: &str) -> Result<String> {
    let mut parts = Vec::new();
    for seg in input.split('/') {
        let seg = seg.trim();
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." || seg.contains('\0') {
            return Err(Error::InvalidPath(input.to_string()));
        }
        parts.push(seg.to_lowercase());
    }
    if parts.is_empty() {
        return Err(Error::InvalidPath(input.to_string()));
    }
    Ok(parts.join("/"))
}

fn parent_of(path: &str) -> Option<String> {
    path.rsplit_once('/').map(|(p, _)| p.to_string())
}

fn title_from_path(path: &str) -> String {
    let slug = path.rsplit('/').next().unwrap_or(path);
    let cleaned = slug.replace(['-', '_'], " ");
    let mut chars = cleaned.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => cleaned,
    }
}

/// Internal album id used by the site's access cookie. Grants nothing alone.
fn generate_token() -> String {
    let bytes: [u8; 6] = rand::thread_rng().gen();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A random secret for link-sharing. This *is* a credential — CSPRNG, 16 bytes,
/// URL-safe, matching the web gallery's `shareToken` semantics.
pub fn generate_share_token() -> String {
    let bytes: [u8; 16] = rand::thread_rng().gen();
    base64_url_safe(&bytes)
}

fn base64_url_safe(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

impl Library {
    pub fn create_album(&self, new: &NewAlbum) -> Result<Album> {
        let path = normalize_path(&new.path)?;
        if self.album_by_path(&path)?.is_some() {
            return Err(Error::AlbumExists(path));
        }
        let title = new
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| title_from_path(&path));

        self.with_conn(|c| {
            c.execute(
                "INSERT INTO albums(path, parent_path, title, description, date, token, \
                 sort, style, is_collection) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    path,
                    parent_of(&path),
                    title,
                    new.description,
                    new.date,
                    generate_token(),
                    "date-desc",
                    "single-column",
                    new.is_collection as i64,
                ],
            )?;
            Ok(())
        })?;

        self.album_by_path(&path)?
            .ok_or_else(|| Error::AlbumNotFound(path))
    }

    pub fn album_by_path(&self, path: &str) -> Result<Option<Album>> {
        let row = self.with_conn(|c| {
            Ok(c.query_row(
                &format!("SELECT {ALBUM_COLS} FROM albums WHERE path = ?1"),
                params![path],
                album_from_row,
            )
            .optional()?)
        })?;

        match row {
            None => Ok(None),
            Some((mut album, cover_id)) => {
                self.hydrate(&mut album, cover_id)?;
                Ok(Some(album))
            }
        }
    }

    /// All albums, ordered the way the gallery orders them: explicit
    /// `sort_order` first, then title.
    pub fn albums(&self) -> Result<Vec<Album>> {
        let rows = self.with_conn(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {ALBUM_COLS} FROM albums \
                 ORDER BY (sort_order IS NULL), sort_order, title"
            ))?;
            let mapped = stmt.query_map([], album_from_row)?;
            let mut out = Vec::new();
            for r in mapped {
                out.push(r?);
            }
            Ok(out)
        })?;

        let mut albums = Vec::with_capacity(rows.len());
        for (mut album, cover_id) in rows {
            self.hydrate(&mut album, cover_id)?;
            albums.push(album);
        }
        Ok(albums)
    }

    /// Direct children of `parent` (pass `None` for the top level).
    pub fn child_albums(&self, parent: Option<&str>) -> Result<Vec<Album>> {
        Ok(self
            .albums()?
            .into_iter()
            .filter(|a| a.parent_path.as_deref() == parent)
            .collect())
    }

    /// Fill in derived fields the row itself doesn't carry.
    fn hydrate(&self, album: &mut Album, cover_photo_id: Option<i64>) -> Result<()> {
        album.tags = self.album_tags(album.id)?;
        album.cover_filename = match cover_photo_id {
            Some(id) => self.with_conn(|c| {
                Ok(c.query_row(
                    "SELECT filename FROM photos WHERE id = ?1",
                    params![id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
            })?,
            None => None,
        };
        Ok(())
    }

    pub fn update_album(&self, path: &str, update: &AlbumUpdate) -> Result<Album> {
        let album = self
            .album_by_path(path)?
            .ok_or_else(|| Error::AlbumNotFound(path.to_string()))?;

        let id = album.id;
        self.with_conn(|c| {
            // `Option<T>` → set when present; `Option<Option<T>>` → the inner
            // `None` clears the column. Untouched fields keep their value.
            if let Some(v) = &update.title {
                c.execute("UPDATE albums SET title = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = &update.description {
                c.execute("UPDATE albums SET description = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = &update.date {
                c.execute("UPDATE albums SET date = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = &update.password {
                c.execute("UPDATE albums SET password = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = &update.share_token {
                c.execute("UPDATE albums SET share_token = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = &update.sort {
                c.execute("UPDATE albums SET sort = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = &update.style {
                c.execute("UPDATE albums SET style = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = &update.cover_photo_id {
                c.execute("UPDATE albums SET cover_photo_id = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = &update.sort_order {
                c.execute("UPDATE albums SET sort_order = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = &update.body {
                c.execute("UPDATE albums SET body = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = update.is_collection {
                c.execute("UPDATE albums SET is_collection = ?1 WHERE id = ?2", params![v as i64, id])?;
            }
            if let Some(v) = update.hidden {
                c.execute("UPDATE albums SET hidden = ?1 WHERE id = ?2", params![v as i64, id])?;
            }
            if let Some(v) = update.allow_download {
                c.execute("UPDATE albums SET allow_download = ?1 WHERE id = ?2", params![v as i64, id])?;
            }
            if let Some(v) = update.proofing {
                c.execute("UPDATE albums SET proofing = ?1 WHERE id = ?2", params![v as i64, id])?;
            }
            Ok(())
        })?;

        if let Some(tags) = &update.tags {
            self.set_album_tags(album.id, tags)?;
        }

        self.album_by_path(path)?
            .ok_or_else(|| Error::AlbumNotFound(path.to_string()))
    }

    /// Rename or move an album, carrying its descendants with it.
    pub fn move_album(&self, from: &str, to: &str) -> Result<Album> {
        let to = normalize_path(to)?;
        if self.album_by_path(from)?.is_none() {
            return Err(Error::AlbumNotFound(from.to_string()));
        }
        if self.album_by_path(&to)?.is_some() {
            return Err(Error::AlbumExists(to));
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(Error::InvalidPath(format!(
                "cannot move {from} inside itself"
            )));
        }

        let descendant_prefix = format!("{from}/");
        self.with_tx(|tx| {
            tx.execute(
                "UPDATE albums SET path = ?1, parent_path = ?2 WHERE path = ?3",
                params![to, parent_of(&to), from],
            )?;

            // Re-path descendants: '<from>/x/y' → '<to>/x/y'
            let mut stmt = tx.prepare("SELECT id, path FROM albums WHERE path LIKE ?1")?;
            let rows = stmt.query_map(params![format!("{descendant_prefix}%")], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut moves = Vec::new();
            for row in rows {
                let (id, old) = row?;
                let suffix = &old[descendant_prefix.len()..];
                moves.push((id, format!("{to}/{suffix}")));
            }
            let mut upd = tx.prepare("UPDATE albums SET path = ?1, parent_path = ?2 WHERE id = ?3")?;
            for (id, new_path) in moves {
                upd.execute(params![new_path, parent_of(&new_path), id])?;
            }
            Ok(())
        })?;

        self.album_by_path(&to)?
            .ok_or_else(|| Error::AlbumNotFound(to))
    }

    /// Delete an album. Photos are untouched — an album is a view over them,
    /// not a container of them.
    pub fn delete_album(&self, path: &str) -> Result<()> {
        let n = self.with_conn(|c| {
            Ok(c.execute("DELETE FROM albums WHERE path = ?1", params![path])?)
        })?;
        if n == 0 {
            return Err(Error::AlbumNotFound(path.to_string()));
        }
        Ok(())
    }

    // ---------------------------------------------------------- membership

    /// Add photos to an album, appending in the given order.
    pub fn add_photos_to_album(&self, album_path: &str, photo_ids: &[i64]) -> Result<usize> {
        let album = self
            .album_by_path(album_path)?
            .ok_or_else(|| Error::AlbumNotFound(album_path.to_string()))?;

        self.with_tx(|tx| {
            let next: i64 = tx.query_row(
                "SELECT IFNULL(MAX(position), -1) + 1 FROM album_photos WHERE album_id = ?1",
                params![album.id],
                |r| r.get(0),
            )?;
            let mut stmt = tx.prepare(
                "INSERT INTO album_photos(album_id, photo_id, position) VALUES(?1,?2,?3) \
                 ON CONFLICT(album_id, photo_id) DO NOTHING",
            )?;
            let mut added = 0;
            for (i, id) in photo_ids.iter().enumerate() {
                added += stmt.execute(params![album.id, id, next + i as i64])?;
            }
            Ok(added)
        })
    }

    pub fn remove_photos_from_album(&self, album_path: &str, photo_ids: &[i64]) -> Result<usize> {
        let album = self
            .album_by_path(album_path)?
            .ok_or_else(|| Error::AlbumNotFound(album_path.to_string()))?;
        self.with_tx(|tx| {
            let mut stmt =
                tx.prepare("DELETE FROM album_photos WHERE album_id = ?1 AND photo_id = ?2")?;
            let mut n = 0;
            for id in photo_ids {
                n += stmt.execute(params![album.id, id])?;
            }
            Ok(n)
        })
    }

    /// Set the explicit order of an album's photos. Ids not listed keep a
    /// position after the listed ones.
    pub fn reorder_album(&self, album_path: &str, photo_ids: &[i64]) -> Result<()> {
        let album = self
            .album_by_path(album_path)?
            .ok_or_else(|| Error::AlbumNotFound(album_path.to_string()))?;
        self.with_tx(|tx| {
            let mut stmt = tx.prepare(
                "UPDATE album_photos SET position = ?1 WHERE album_id = ?2 AND photo_id = ?3",
            )?;
            for (i, id) in photo_ids.iter().enumerate() {
                stmt.execute(params![i as i64, album.id, id])?;
            }
            Ok(())
        })
    }

    /// Persist sibling ordering (what the gallery reads as `order`).
    pub fn reorder_siblings(&self, paths: &[String]) -> Result<()> {
        self.with_tx(|tx| {
            let mut stmt = tx.prepare("UPDATE albums SET sort_order = ?1 WHERE path = ?2")?;
            for (i, path) in paths.iter().enumerate() {
                stmt.execute(params![(i + 1) as i64, path])?;
            }
            Ok(())
        })
    }

    /// Photos in an album, in album order.
    pub fn album_photos(&self, album_path: &str) -> Result<Vec<crate::model::Photo>> {
        self.photos(&crate::model::PhotoFilter {
            album_path: Some(album_path.to_string()),
            sort: crate::model::PhotoSort::AlbumOrder,
            ..Default::default()
        })
    }

    // ---------------------------------------------------------------- tags

    pub fn album_tags(&self, album_id: i64) -> Result<Vec<String>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT t.name FROM tags t JOIN album_tags at ON at.tag_id = t.id \
                 WHERE at.album_id = ?1 ORDER BY t.name",
            )?;
            let rows = stmt.query_map(params![album_id], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    pub fn set_album_tags(&self, album_id: i64, tags: &[String]) -> Result<()> {
        self.with_tx(|tx| {
            tx.execute("DELETE FROM album_tags WHERE album_id = ?1", params![album_id])?;
            let mut insert_tag =
                tx.prepare("INSERT INTO tags(name) VALUES(?1) ON CONFLICT(name) DO NOTHING")?;
            let mut find_tag = tx.prepare("SELECT id FROM tags WHERE name = ?1")?;
            let mut link = tx.prepare(
                "INSERT INTO album_tags(album_id, tag_id) VALUES(?1,?2) \
                 ON CONFLICT DO NOTHING",
            )?;
            for tag in tags {
                let tag = tag.trim();
                if tag.is_empty() {
                    continue;
                }
                insert_tag.execute(params![tag])?;
                let tag_id: i64 = find_tag.query_row(params![tag], |r| r.get(0))?;
                link.execute(params![album_id, tag_id])?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> Library {
        Library::open_in_memory("/tmp/lib").unwrap()
    }

    fn new_album(path: &str) -> NewAlbum {
        NewAlbum {
            path: path.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize_path("2026/Weddings/Ana").unwrap(), "2026/weddings/ana");
        assert_eq!(normalize_path("/a/b/").unwrap(), "a/b");
        assert!(normalize_path("../etc").is_err());
        assert!(normalize_path("").is_err());
    }

    #[test]
    fn derives_title_from_slug() {
        let l = lib();
        let a = l.create_album(&new_album("2026/ana-i-ivan")).unwrap();
        assert_eq!(a.title, "Ana i ivan");
        assert_eq!(a.parent_path.as_deref(), Some("2026"));
        assert!(!a.token.is_empty());
    }

    #[test]
    fn rejects_duplicate_album() {
        let l = lib();
        l.create_album(&new_album("a")).unwrap();
        assert!(matches!(
            l.create_album(&new_album("a")),
            Err(Error::AlbumExists(_))
        ));
    }

    #[test]
    fn update_merges_and_clears() {
        let l = lib();
        l.create_album(&new_album("a")).unwrap();

        let updated = l
            .update_album(
                "a",
                &AlbumUpdate {
                    description: Some(Some("hello".into())),
                    password: Some(Some("secret".into())),
                    proofing: Some(true),
                    tags: Some(vec!["wedding".into(), "ljeto".into()]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.description.as_deref(), Some("hello"));
        assert!(updated.is_locked());
        assert!(updated.proofing);
        assert_eq!(updated.tags, vec!["ljeto", "wedding"]);

        // A partial update must not disturb untouched fields...
        let again = l
            .update_album(
                "a",
                &AlbumUpdate {
                    title: Some("New Title".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(again.title, "New Title");
        assert_eq!(again.description.as_deref(), Some("hello"), "kept");
        assert!(again.proofing, "kept");

        // ...and Some(None) explicitly clears.
        let cleared = l
            .update_album(
                "a",
                &AlbumUpdate {
                    password: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(cleared.password.is_none());
        assert!(!cleared.is_locked());
    }

    #[test]
    fn share_tokens_are_random_and_urlsafe() {
        let a = generate_share_token();
        let b = generate_share_token();
        assert_ne!(a, b);
        assert!(a.len() >= 20);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn moves_album_with_descendants() {
        let l = lib();
        l.create_album(&new_album("2026")).unwrap();
        l.create_album(&new_album("2026/ana")).unwrap();
        l.create_album(&new_album("2026/ana/day2")).unwrap();

        l.move_album("2026/ana", "2027/ana-ivan").unwrap();

        assert!(l.album_by_path("2026/ana").unwrap().is_none());
        assert!(l.album_by_path("2027/ana-ivan").unwrap().is_some());
        let moved_child = l.album_by_path("2027/ana-ivan/day2").unwrap().unwrap();
        assert_eq!(moved_child.parent_path.as_deref(), Some("2027/ana-ivan"));
    }

    #[test]
    fn refuses_move_into_self() {
        let l = lib();
        l.create_album(&new_album("a")).unwrap();
        assert!(l.move_album("a", "a/b").is_err());
    }

    #[test]
    fn child_albums_are_scoped_to_parent() {
        let l = lib();
        l.create_album(&new_album("2026")).unwrap();
        l.create_album(&new_album("2026/a")).unwrap();
        l.create_album(&new_album("2026/b")).unwrap();
        l.create_album(&new_album("2027")).unwrap();

        let top = l.child_albums(None).unwrap();
        assert_eq!(top.len(), 2);
        let kids = l.child_albums(Some("2026")).unwrap();
        assert_eq!(kids.len(), 2);
    }

    #[test]
    fn sibling_order_drives_listing() {
        let l = lib();
        l.create_album(&new_album("b")).unwrap();
        l.create_album(&new_album("a")).unwrap();
        // Alphabetical by default
        assert_eq!(l.albums().unwrap()[0].path, "a");
        // Explicit order wins
        l.reorder_siblings(&["b".into(), "a".into()]).unwrap();
        assert_eq!(l.albums().unwrap()[0].path, "b");
    }
}

#[cfg(test)]
mod serde_tests {
    use super::*;

    /// The UI must be able to say "leave it alone" and "clear it" distinctly.
    #[test]
    fn absent_field_differs_from_explicit_null() {
        let untouched: AlbumUpdate = serde_json::from_str(r#"{"title":"X"}"#).unwrap();
        assert_eq!(untouched.title.as_deref(), Some("X"));
        assert!(untouched.password.is_none(), "absent = leave alone");

        let cleared: AlbumUpdate = serde_json::from_str(r#"{"password":null}"#).unwrap();
        assert_eq!(cleared.password, Some(None), "null = clear");

        let set: AlbumUpdate = serde_json::from_str(r#"{"password":"s3cret"}"#).unwrap();
        assert_eq!(set.password, Some(Some("s3cret".into())));
    }

    #[test]
    fn new_album_accepts_camel_case_from_the_ui() {
        let a: NewAlbum =
            serde_json::from_str(r#"{"path":"2026/x","isCollection":true}"#).unwrap();
        assert_eq!(a.path, "2026/x");
        assert!(a.is_collection);
        assert!(a.title.is_none());
    }
}
