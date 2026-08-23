//! Album tree, membership and ordering.
//!
//! Albums mirror the folder structure the web gallery publishes:
//! `2026/weddings/ana-ivan`. Every field maps to gallery frontmatter — see
//! [`crate::publish`] for the contract.
//!
//! # The path is the identity
//!
//! An album is named by its path, and so is everything that refers to one: the
//! published folder, the URL a client was sent, the sync subscription, the row
//! in `published_files`. There is a numeric `id` in the table, but it is a
//! join key, not a name. That is why [`Library::move_album`] is more than an
//! `UPDATE`: a rename has to carry the descendants and their subscriptions with
//! it, and even then it cannot reach the copy already sitting on the server.
//!
//! # An album is a view, not a container
//!
//! Photos belong to the library; an album lists some of them in an order. So
//! deleting an album deletes no photographs, one photo may appear in several
//! albums, and the folder a file happens to live in on disk has nothing to do
//! with which album publishes it.

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
    /// Slash-separated, run through [`normalize_path`] before use, so a path
    /// typed with capitals or stray slashes is accepted and lowercased. Any
    /// folders above it that the catalog does not have yet are created as
    /// collections — see [`Library::ensure_collection_chain`].
    pub path: String,
    /// Absent or blank derives one from the last path segment, dashes and
    /// underscores becoming spaces: `ana-i-ivan` → "Ana i ivan". A derived
    /// title is a placeholder, not a decision, and it is what a client sees at
    /// the top of the gallery until someone changes it.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// The shoot date shown in the gallery, as free text — nothing here parses
    /// or validates it, and it is unrelated to any EXIF capture date.
    #[serde(default)]
    pub date: Option<String>,
    /// A folder of albums rather than an album of photographs. The gallery
    /// draws one as a grid of its children and never renders loose photos in
    /// it, which is why [`Library::add_photos_to_album`] refuses one outright.
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
    /// Internal album id. Only set when adopting an album from another machine —
    /// the id must match across machines or the access cookie breaks.
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub date: Option<Option<String>>,
    /// The gallery's password, stored and published in plain text — that is the
    /// web side's design, not an oversight here, and it means a catalog and a
    /// published `index.md` both hold every client's password in the clear.
    /// Clearing it (`Some(None)`) makes the album public the next time it is
    /// published, with no further confirmation anywhere.
    #[serde(default, deserialize_with = "double_option")]
    pub password: Option<Option<String>>,
    /// The secret in a share link. This *is* a credential: generate it with
    /// [`generate_share_token`] rather than letting anyone choose one, because
    /// a guessable value is the whole of the protection on a link-shared album.
    #[serde(default, deserialize_with = "double_option")]
    pub share_token: Option<Option<String>>,
    /// How the gallery orders the photos: `date-desc` and friends, or `custom`,
    /// which is the only value that makes the published `photoOrder` list mean
    /// anything. Setting `custom` without also ordering the album leaves the
    /// client looking at whatever order the rows happen to be in.
    #[serde(default)]
    pub sort: Option<String>,
    /// The gallery layout — `single-column`, `grid`, `masonry`, `slideshow`.
    #[serde(default)]
    pub style: Option<String>,
    /// The cover photo, by catalog id. It has to be a photo the album actually
    /// publishes: [`crate::publish::render_frontmatter`] drops a cover that is
    /// not among them rather than emit a `thumbnail` pointing at a file the
    /// gallery would 404 on, and the site falls back to the first photo.
    #[serde(default, deserialize_with = "double_option")]
    pub cover_photo_id: Option<Option<i64>>,
    /// Turning an album with photos into a collection does not move them; it
    /// makes the gallery stop drawing them.
    #[serde(default)]
    pub is_collection: Option<bool>,
    /// Keep the album out of listings. It stays reachable by anyone holding the
    /// URL — this is tidiness, not access control; that is `password` and
    /// `share_token`.
    #[serde(default)]
    pub hidden: Option<bool>,
    /// Whether the gallery offers the ZIP of originals. Off by default, and the
    /// site's download endpoint refuses without it.
    #[serde(default)]
    pub allow_download: Option<bool>,
    /// Turn on client proofing: hearts on every photo and a review panel whose
    /// submissions land in the album's server-owned `.meta/` folder. Publishing
    /// never touches that folder, so selections survive a republish.
    #[serde(default)]
    pub proofing: Option<bool>,
    /// Rank among siblings, published as the gallery's `order`. Lower first;
    /// cleared (`Some(None)`) sorts the album after every ranked sibling, by
    /// title. Normally written wholesale by [`Library::reorder_siblings`].
    #[serde(default, deserialize_with = "double_option")]
    pub sort_order: Option<Option<i64>>,
    /// Prose shown under the album. Published as a separate `body.md`, not as
    /// part of the frontmatter, and clearing it deletes that file.
    #[serde(default, deserialize_with = "double_option")]
    pub body: Option<Option<String>>,
    /// The album's complete tag set, not additions: whatever is sent replaces
    /// what is there, so a form that renders only some of the tags and posts
    /// them back silently deletes the rest. Omit the field to leave tags alone.
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

/// Order albums so that every parent comes immediately before its children.
///
/// The key for one album is the sibling key of each ancestor followed by its
/// own. Because a parent's key is a prefix of its children's, and a shorter
/// vector sorts before a longer one that starts the same way, the result is a
/// depth-first walk of the tree with siblings in gallery order.
fn sort_as_tree(albums: &mut [Album]) {
    use std::collections::HashMap;

    // Sibling key: ordered albums first, in their order; the rest by title.
    let keys: HashMap<&str, (i64, String)> = albums
        .iter()
        .map(|a| {
            (
                a.path.as_str(),
                (a.sort_order.unwrap_or(i64::MAX), a.title.to_lowercase()),
            )
        })
        .collect();

    let key_of = |path: &str| -> Vec<(i64, String, String)> {
        let mut prefix = String::new();
        let mut out = Vec::new();
        for segment in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            // A gap in the chain — an album whose parent folder is not itself
            // catalogued — still sorts sensibly under its own slug.
            let (order, title) = keys
                .get(prefix.as_str())
                .cloned()
                .unwrap_or((i64::MAX, segment.to_lowercase()));
            out.push((order, title, segment.to_lowercase()));
        }
        out
    };

    let mut decorated: Vec<_> = albums
        .iter()
        .map(|a| (key_of(&a.path), a.path.clone()))
        .collect();
    decorated.sort();

    let position: HashMap<&str, usize> = decorated
        .iter()
        .enumerate()
        .map(|(i, (_, path))| (path.as_str(), i))
        .collect();
    albums.sort_by_key(|a| position[a.path.as_str()]);
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
    /// Create an album, and any collection above it that does not exist yet.
    ///
    /// The new row gets a fresh `token` — the id the gallery's access cookie
    /// names this album by. It is generated once and then never regenerated:
    /// changing it invalidates every unlock a client is currently holding, so
    /// an album adopted from another machine must be given that machine's token
    /// through [`AlbumUpdate::token`] rather than being created afresh.
    ///
    /// Fails if the path is already taken — this never merges into an existing
    /// album.
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

        // The folders above this album have to exist as collections, or the
        // gallery cannot navigate to it. Recursion terminates: each call is one
        // segment shorter, and a one-segment path has no ancestors.
        self.ensure_collection_chain(&path)?;

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

    /// Create a collection row for every missing folder above `path`.
    ///
    /// The gallery navigates by folder: `/photos` lists direct children of the
    /// root, each collection lists its own. An album at `2026/weddings/ana` with
    /// no `2026` and no `2026/weddings` is reachable only by typing its URL, and
    /// access inheritance has no ancestor to inherit from. So the chain is not
    /// decoration — it is what makes the album part of the site.
    ///
    /// Returns the paths it created, shallowest first.
    pub fn ensure_collection_chain(&self, path: &str) -> Result<Vec<String>> {
        let path = normalize_path(path)?;
        let segments: Vec<&str> = path.split('/').collect();
        let mut created = Vec::new();

        // Every prefix except the path itself — the leaf is the caller's own.
        for depth in 1..segments.len() {
            let ancestor = segments[..depth].join("/");
            if self.album_by_path(&ancestor)?.is_some() {
                continue;
            }
            self.create_album(&NewAlbum {
                path: ancestor.clone(),
                is_collection: true,
                ..Default::default()
            })?;
            created.push(ancestor);
        }
        Ok(created)
    }

    /// One album, fully hydrated — tags and cover filename included, which the
    /// row itself does not carry.
    ///
    /// The path is matched exactly, *not* normalised: pass what
    /// [`normalize_path`] produced, or a path a user typed with a capital in it
    /// comes back as "no such album" for a folder that is plainly there.
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

    /// All albums, in tree order.
    ///
    /// Siblings are ordered the way the gallery orders them — explicit
    /// `sort_order` first, then title — but a plain `ORDER BY title` is not
    /// enough on its own: it interleaves the levels, so an album can be listed
    /// above the folder that contains it and the indentation a sidebar draws
    /// stops meaning anything. Every album is therefore keyed by the sibling
    /// key of each of its ancestors as well as its own, which puts a parent
    /// immediately before its children and keeps whole branches together.
    pub fn albums(&self) -> Result<Vec<Album>> {
        let rows = self.with_conn(|c| {
            let mut stmt = c.prepare(&format!("SELECT {ALBUM_COLS} FROM albums"))?;
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
        sort_as_tree(&mut albums);
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

    /// Apply a partial update and return the album as it now stands.
    ///
    /// Merge semantics, per [`AlbumUpdate`]: an omitted field is left alone, so
    /// a caller need only send what it knows about. The one field that does not
    /// merge is `tags`, which replaces the whole set.
    ///
    /// Nothing is published by this. The client's gallery still shows the old
    /// title, the old password still opens it, and it stays that way until the
    /// album is published and synced.
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
            if let Some(v) = &update.token {
                c.execute("UPDATE albums SET token = ?1 WHERE id = ?2", params![v, id])?;
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

    /// Rename or move an album, carrying its descendants — and their sync
    /// subscriptions — with it.
    ///
    /// This moves nothing on a remote. A tracked album that has already been
    /// pushed stays on the server under its old path as well, and the next
    /// sync publishes it under the new one, so the server ends up holding both.
    /// Removing the old copy is a deletion, and deletions are never implicit
    /// here: the photographer does it deliberately, with `allow_deletes`.
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

        // Moving into a folder that doesn't exist yet must create it, same as
        // creating an album there would.
        self.ensure_collection_chain(&to)?;

        let descendant_prefix = format!("{from}/");
        self.with_tx(|tx| {
            tx.execute(
                "UPDATE albums SET path = ?1, parent_path = ?2 WHERE path = ?3",
                params![to, parent_of(&to), from],
            )?;

            // Re-path descendants: '<from>/x/y' → '<to>/x/y'
            //
            // Compared as a literal prefix, not with LIKE: `_` and `%` are legal
            // in a folder name and are SQL wildcards, so `LIKE 'a_b/%'` also
            // matched `axb/…` and moved an album that had nothing to do with
            // this one. `substr` counts characters, so a match here means the
            // byte prefix is identical too, which is what the slice below needs.
            let mut stmt =
                tx.prepare("SELECT id, path FROM albums WHERE substr(path, 1, ?2) = ?1")?;
            let rows = stmt.query_map(
                params![descendant_prefix, descendant_prefix.chars().count() as i64],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )?;
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

            // Carry the sync subscriptions along. A subscription names an album
            // by path, so leaving them behind means a rename quietly stops
            // syncing the album — no error, just a client's gallery that never
            // updates again — and leaves a row pointing at nothing for every
            // later sync to trip over. Same literal-prefix comparison as above,
            // for the same reason.
            tx.execute(
                "UPDATE album_sync SET album_path = ?1 WHERE album_path = ?2",
                params![to, from],
            )?;
            let mut stmt = tx.prepare(
                "SELECT album_path FROM album_sync WHERE substr(album_path, 1, ?2) = ?1",
            )?;
            let rows = stmt.query_map(
                params![descendant_prefix, descendant_prefix.chars().count() as i64],
                |r| r.get::<_, String>(0),
            )?;
            let mut sub_moves = Vec::new();
            for row in rows {
                let old = row?;
                let suffix = old[descendant_prefix.len()..].to_string();
                sub_moves.push((old, format!("{to}/{suffix}")));
            }
            let mut upd_sub =
                tx.prepare("UPDATE album_sync SET album_path = ?1 WHERE album_path = ?2")?;
            for (old, new_path) in sub_moves {
                upd_sub.execute(params![new_path, old])?;
            }
            Ok(())
        })?;

        self.album_by_path(&to)?
            .ok_or_else(|| Error::AlbumNotFound(to))
    }

    /// Delete an album. Photos are untouched — an album is a view over them,
    /// not a container of them.
    pub fn delete_album(&self, path: &str) -> Result<()> {
        let n = self.with_tx(|tx| {
            let n = tx.execute("DELETE FROM albums WHERE path = ?1", params![path])?;
            // The subscription goes with it. Left behind it names an album the
            // catalog no longer has, and every later sync has to work around a
            // row describing nothing.
            if n > 0 {
                tx.execute("DELETE FROM album_sync WHERE album_path = ?1", params![path])?;
            }
            Ok(n)
        })?;
        if n == 0 {
            return Err(Error::AlbumNotFound(path.to_string()));
        }
        Ok(())
    }

    // ---------------------------------------------------------- membership

    /// Add photos to an album, appending in the given order.
    ///
    /// A collection is refused: the gallery renders one as a grid of its
    /// sub-albums and never draws loose photos, so accepting them here would
    /// mean photos that are in the catalog, published, and invisible.
    pub fn add_photos_to_album(&self, album_path: &str, photo_ids: &[i64]) -> Result<usize> {
        let album = self
            .album_by_path(album_path)?
            .ok_or_else(|| Error::AlbumNotFound(album_path.to_string()))?;
        if album.is_collection {
            return Err(Error::Other(format!(
                "{album_path} is a folder of albums — put the photos in an album inside it"
            )));
        }

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

    /// Take photos out of an album. Returns how many memberships were actually
    /// removed; ids that were not in it are not an error.
    ///
    /// The photographs themselves are untouched — this is a view, not a
    /// container. The published copy on the site *is* affected, but not yet:
    /// the next publish of this album prunes it, because it is in the record of
    /// what this library put there.
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

    /// Set the explicit order of an album's photos. Ids not listed follow the
    /// listed ones, keeping the order they already had among themselves.
    ///
    /// Renumbering the rest is the whole job, not a tidy-up. Left where they
    /// were, they hold positions the listed photos have just been given, and a
    /// tie is settled by filename — so a photo nobody moved can sit ahead of
    /// one that was deliberately placed first. A pull is where this arrives: a
    /// server's `photoOrder` may name only part of an album, and the gallery
    /// puts the photos it does not name after the ones it does, so anything
    /// else republishes the client's gallery in an order nobody chose.
    pub fn reorder_album(&self, album_path: &str, photo_ids: &[i64]) -> Result<()> {
        let album = self
            .album_by_path(album_path)?
            .ok_or_else(|| Error::AlbumNotFound(album_path.to_string()))?;
        let listed: std::collections::BTreeSet<i64> = photo_ids.iter().copied().collect();
        self.with_tx(|tx| {
            let rest: Vec<i64> = {
                // Same ordering the grid reads back, so "kept their order"
                // means what the photographer was looking at.
                let mut stmt = tx.prepare(
                    "SELECT ap.photo_id FROM album_photos ap \
                     JOIN photos p ON p.id = ap.photo_id \
                     WHERE ap.album_id = ?1 ORDER BY ap.position ASC, p.filename ASC",
                )?;
                let rows = stmt.query_map(params![album.id], |r| r.get::<_, i64>(0))?;
                let mut out = Vec::new();
                for row in rows {
                    let id = row?;
                    if !listed.contains(&id) {
                        out.push(id);
                    }
                }
                out
            };

            let mut stmt = tx.prepare(
                "UPDATE album_photos SET position = ?1 WHERE album_id = ?2 AND photo_id = ?3",
            )?;
            for (i, id) in photo_ids.iter().chain(rest.iter()).enumerate() {
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

    /// An album's tags, alphabetical. Keyed by row id, not path — this is the
    /// hydration helper; ordinary callers read `Album::tags`.
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

    /// Replace an album's tags with exactly this list.
    ///
    /// Not additive — every existing tag is dropped first, so passing an empty
    /// slice untags the album. Blank entries are skipped and each name is
    /// trimmed, because the tag is what the gallery builds a `/photos/tags/…`
    /// page from and " weddings" and "weddings" would be two of them.
    ///
    /// Tags are shared rows: unlinking the last album from one leaves the name
    /// in the `tags` table, harmlessly.
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

    /// A catalog row is all these tests need — no pixels involved.
    fn insert_photo(lib: &Library, filename: &str) -> i64 {
        lib.with_conn(|c| {
            c.execute(
                "INSERT INTO photos(rel_path, filename, content_hash, file_size, \
                 mtime_ms, imported_at) VALUES(?1, ?2, 'h', 0, 0, '')",
                params![format!("a/{filename}"), filename],
            )?;
            Ok(c.last_insert_rowid())
        })
        .unwrap()
    }

    /// The sidebar draws this list as a tree, so a parent must never appear
    /// below its own child. Ordering by title alone did exactly that.
    #[test]
    fn albums_come_back_in_tree_order() {
        let lib = lib();
        // Created out of order, and named so that a title sort would scatter
        // them: "Ana" sorts before "Weddings", though it lives inside it.
        // Creating a deep path also creates the folders above it.
        for path in ["2026/weddings/ana-ivan", "2025/zeta"] {
            lib.create_album(&new_album(path)).unwrap();
        }

        let paths: Vec<_> = lib.albums().unwrap().into_iter().map(|a| a.path).collect();
        assert_eq!(
            paths,
            vec!["2025", "2025/zeta", "2026", "2026/weddings", "2026/weddings/ana-ivan"],
            "every parent must come immediately before its children"
        );
    }

    /// Explicit order still decides between siblings, and only between them.
    #[test]
    fn sort_order_ranks_siblings_without_breaking_the_tree() {
        let lib = lib();
        for path in ["b", "b/child", "a"] {
            lib.create_album(&new_album(path)).unwrap();
        }
        // Push "b" ahead of "a" by hand.
        lib.update_album(
            "b",
            &AlbumUpdate {
                sort_order: Some(Some(1)),
                ..Default::default()
            },
        )
        .unwrap();

        let paths: Vec<_> = lib.albums().unwrap().into_iter().map(|a| a.path).collect();
        assert_eq!(paths, vec!["b", "b/child", "a"]);
    }

    /// Photos in a collection would be published and then never drawn, since
    /// the gallery renders a collection as a grid of its sub-albums.
    #[test]
    fn photos_cannot_be_added_to_a_folder_of_albums() {
        let lib = lib();
        lib.create_album(&new_album("2026/weddings/ana-ivan")).unwrap();
        let err = lib.add_photos_to_album("2026/weddings", &[1]).unwrap_err();
        assert!(
            err.to_string().contains("folder of albums"),
            "unhelpful error: {err}"
        );
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

    /// The gallery navigates by folder, so the folders have to exist. Creating
    /// a deep album creates them, and a second album in the same tree reuses
    /// them rather than failing on the duplicate.
    #[test]
    fn creating_a_deep_album_creates_the_folders_above_it() {
        let l = lib();
        l.create_album(&new_album("2026/weddings/ana-ivan")).unwrap();

        let year = l.album_by_path("2026").unwrap().expect("2026 exists");
        let weddings = l
            .album_by_path("2026/weddings")
            .unwrap()
            .expect("2026/weddings exists");
        assert!(year.is_collection);
        assert!(weddings.is_collection);
        assert_eq!(weddings.parent_path.as_deref(), Some("2026"));
        assert_eq!(weddings.title, "Weddings");

        // A sibling reuses the same folders.
        l.create_album(&new_album("2026/weddings/mia-luka")).unwrap();
        assert_eq!(l.albums().unwrap().len(), 4);

        // Moving into a new branch creates that branch too.
        l.move_album("2026/weddings/mia-luka", "2027/spring/mia-luka")
            .unwrap();
        assert!(l.album_by_path("2027/spring").unwrap().unwrap().is_collection);
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

    /// `_` is a legal separator in an album folder, and it is also SQL's
    /// single-character wildcard. Matching descendants with `LIKE` therefore
    /// let a move reach into an album it has nothing to do with.
    #[test]
    fn moving_an_album_with_an_underscore_leaves_its_neighbours_where_they_are() {
        let l = lib();
        l.create_album(&new_album("a_b")).unwrap();
        l.create_album(&new_album("a_b/mine")).unwrap();
        l.create_album(&new_album("axb")).unwrap();
        l.create_album(&new_album("axb/theirs")).unwrap();

        l.move_album("a_b", "moved").unwrap();

        assert!(l.album_by_path("moved/mine").unwrap().is_some(), "its own child came along");
        assert!(
            l.album_by_path("axb/theirs").unwrap().is_some(),
            "an unrelated album must not be dragged along by a wildcard match"
        );
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

    /// A reorder that names only some of an album's photos has to move the
    /// rest out of the way, not leave them sitting on the positions it just
    /// handed out.
    ///
    /// A pull is where this arrives: the server's `photoOrder` may cover only
    /// part of an album, and the gallery puts the photos it does not name
    /// *after* the ones it does. A catalog that instead leaves them tied with
    /// the named ones — settled by filename, so often ahead of them —
    /// republishes the album in an order neither the photographer nor the
    /// server ever chose, and the client's gallery reshuffles itself.
    #[test]
    fn reordering_part_of_an_album_moves_the_rest_behind_it() {
        let l = lib();
        l.create_album(&new_album("a")).unwrap();
        let ids: Vec<i64> = ["one.jpg", "two.jpg", "three.jpg"]
            .iter()
            .map(|name| insert_photo(&l, name))
            .collect();
        l.add_photos_to_album("a", &ids).unwrap();

        // Only the last photo is placed; the other two are not mentioned.
        l.reorder_album("a", &[ids[2]]).unwrap();

        let order: Vec<String> = l
            .album_photos("a")
            .unwrap()
            .into_iter()
            .map(|p| p.filename)
            .collect();
        assert_eq!(order, vec!["three.jpg", "one.jpg", "two.jpg"]);
    }

    /// And the unnamed photos keep the order they already had among
    /// themselves — a drag that touches two frames must not throw away the
    /// arrangement of the other three hundred.
    #[test]
    fn a_partial_reorder_preserves_the_arrangement_of_what_it_does_not_name() {
        let l = lib();
        l.create_album(&new_album("a")).unwrap();
        let ids: Vec<i64> = ["one.jpg", "two.jpg", "three.jpg", "four.jpg"]
            .iter()
            .map(|name| insert_photo(&l, name))
            .collect();
        l.add_photos_to_album("a", &ids).unwrap();
        // An existing arrangement: reversed.
        l.reorder_album("a", &[ids[3], ids[2], ids[1], ids[0]]).unwrap();

        // Now pull one frame to the front.
        l.reorder_album("a", &[ids[1]]).unwrap();

        let order: Vec<String> = l
            .album_photos("a")
            .unwrap()
            .into_iter()
            .map(|p| p.filename)
            .collect();
        assert_eq!(order, vec!["two.jpg", "four.jpg", "three.jpg", "one.jpg"]);
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
