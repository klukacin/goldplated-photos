//! `gpp` — command-line driver for the core.
//!
//! Exists for three reasons: it exercises the whole core without a GUI, it
//! automates work the desktop app would otherwise have to babysit, and it is
//! the bridge the existing web admin can shell out to.
//!
//! Arguments are parsed by hand to keep the dependency surface (and therefore
//! the licence audit) as small as possible.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gpp_core::albums::{AlbumUpdate, NewAlbum};
use gpp_core::develop::EditOp;
use gpp_core::import::{import_dir, ImportOptions};
use gpp_core::model::{Flag, PhotoFilter, PhotoSort};
use gpp_core::publish::{publish_album, PublishOptions};
use gpp_core::remotes::RemoteUpdate;
use gpp_core::sync::{SyncDirection, SyncScopeKind};
use gpp_core::{remote, sync, Library, Result};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "help" || args[0] == "--help" || args[0] == "-h" {
        print_usage();
        return ExitCode::SUCCESS;
    }

    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    let cmd = args[0].as_str();
    let rest = &args[1..];

    match cmd {
        "init" => cmd_init(rest),
        "import" => cmd_import(rest),
        "ls" => cmd_ls(rest),
        "rate" => cmd_rate(rest),
        "flag" => cmd_flag(rest),
        "album" => cmd_album(rest),
        "lr" => cmd_lr(rest),
        "develop" => cmd_develop(rest),
        "publish" => cmd_publish(rest),
        "sync" => cmd_sync(rest),
        "remote" => cmd_remote(rest),
        "remotes" => cmd_remotes(rest),
        "sources" => cmd_sources(rest),
        "pull" => cmd_pull(rest),
        "push" => cmd_push(rest),
        "push-to" => cmd_push_to(rest),
        "xmp" => cmd_xmp(rest),
        "stats" => cmd_stats(rest),
        other => Err(gpp_core::Error::other(format!(
            "unknown command '{other}' — run `gpp help`"
        ))),
    }
}

// ------------------------------------------------------------------ helpers

/// `--flag value` lookup.
fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let idx = args.iter().position(|a| a == name)?;
    args.get(idx + 1).map(|s| s.as_str())
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// First non-flag argument.
fn positional(args: &[String]) -> Option<&str> {
    let mut skip_next = false;
    for (i, a) in args.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if a.starts_with("--") {
            // Flags that take a value consume the next token.
            let takes_value = !matches!(
                a.as_str(),
                "--recursive" | "--no-thumbs" | "--force" | "--json" | "--apply" | "--allow-deletes"
                    | "--push" | "--pull" | "--both" | "--record" | "--collection"
                    | "--share-link" | "--no-share-link" | "--proofing" | "--no-proofing"
                    | "--allow-download" | "--metadata-only" | "--include-rejected"
                    | "--no-recursive" | "--bw" | "--no-bw" | "--flip-h" | "--flip-v"
                    | "--reset" | "--show" | "--dry-run" | "--drop-photos"
            );
            skip_next = takes_value;
            continue;
        }
        return Some(&args[i]);
    }
    None
}

/// Library root: `--library <path>`, else `GPP_LIBRARY`, else the cwd.
fn library_root(args: &[String]) -> Result<PathBuf> {
    if let Some(p) = opt(args, "--library") {
        return Ok(PathBuf::from(p));
    }
    if let Ok(p) = std::env::var("GPP_LIBRARY") {
        return Ok(PathBuf::from(p));
    }
    std::env::current_dir().map_err(gpp_core::Error::PlainIo)
}

fn open_library(args: &[String]) -> Result<Library> {
    Library::open(library_root(args)?)
}

// ----------------------------------------------------------------- commands

fn cmd_init(args: &[String]) -> Result<()> {
    let root = match positional(args) {
        Some(p) => PathBuf::from(p),
        None => library_root(args)?,
    };
    let lib = Library::open(&root)?;
    println!("Initialised library at {}", lib.root().display());
    println!("Catalog: {}", lib.gpp_dir().join("catalog.db").display());
    Ok(())
}

fn cmd_import(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let dir = match positional(args) {
        Some(p) => PathBuf::from(p),
        None => lib.root().to_path_buf(),
    };

    let opts = ImportOptions {
        recursive: !has(args, "--no-recursive"),
        generate_thumbnails: !has(args, "--no-thumbs"),
        force: has(args, "--force"),
    };

    let last = std::sync::Mutex::new(0usize);
    let summary = import_dir(
        &lib,
        &dir,
        &opts,
        Some(&|p| {
            // One line per 25 files keeps the output readable on big imports.
            let mut last = last.lock().unwrap();
            if p.processed == p.total || p.processed >= *last + 25 {
                *last = p.processed;
                eprintln!("  [{}/{}] {}", p.processed, p.total, p.current);
            }
        }),
        // No stop signal: Ctrl-C already ends a one-shot command, and the
        // graceful variant would cost a signal-handling dependency to save a
        // partial import nobody asked this tool for.
        None,
    )?;

    // Pointing this at a camera card copies it into the library. Saying so
    // matters more here than in the app: there is no sidebar to notice a new
    // folder in, so the only way a CLI user learns their library grew by a
    // gigabyte is if this line tells them.
    if let Some(into) = &summary.copied_into {
        println!("copied {} file(s) into {into}", summary.copied_in);
    }

    println!(
        "imported {} · updated {} · unchanged {} · duplicates {} · failed {}",
        summary.imported,
        summary.updated,
        summary.skipped,
        summary.duplicates,
        summary.failed.len()
    );
    for (path, err) in &summary.failed {
        eprintln!("  failed: {path}: {err}");
    }
    // A registered source that is not attached is skipped, not failed — and a
    // run that says nothing about it looks exactly like one that found nothing
    // new there.
    for note in &summary.notes {
        eprintln!("  note: {note}");
    }

    // Catalogued but with no readable pixels — a corrupt file, or a RAW format
    // this build does not develop. They will not be published.
    if !summary.undecodable.is_empty() {
        eprintln!("  {} file(s) have no preview:", summary.undecodable.len());
        for path in &summary.undecodable {
            eprintln!("    {path}");
        }
    }
    Ok(())
}

fn cmd_ls(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let filter = PhotoFilter {
        min_rating: opt(args, "--min-rating").and_then(|v| v.parse().ok()),
        flag: opt(args, "--flag").map(Flag::parse),
        album_path: opt(args, "--album").map(String::from),
        camera_model: opt(args, "--camera").map(String::from),
        text: opt(args, "--search").map(String::from),
        captured_from: opt(args, "--from").map(String::from),
        captured_to: opt(args, "--to").map(String::from),
        limit: opt(args, "--limit").and_then(|v| v.parse().ok()).or(Some(50)),
        sort: match opt(args, "--sort") {
            Some("name") => PhotoSort::NameAsc,
            Some("rating") => PhotoSort::RatingDesc,
            Some("newest") => PhotoSort::CapturedDesc,
            Some("album") => PhotoSort::AlbumOrder,
            _ => PhotoSort::CapturedAsc,
        },
        ..Default::default()
    };

    let photos = lib.photos(&filter)?;
    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&photos)?);
        return Ok(());
    }

    for p in &photos {
        let stars: String = "★".repeat(p.rating as usize);
        let flag = match p.flag {
            Flag::Pick => " [pick]",
            Flag::Reject => " [reject]",
            Flag::None => "",
        };
        println!(
            "{:>6}  {:<40} {:<5}{}  {}",
            p.id,
            p.rel_path,
            stars,
            flag,
            p.captured_at.as_deref().unwrap_or("-")
        );
    }
    println!("({} photos)", photos.len());
    Ok(())
}

fn cmd_rate(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let ids = parse_ids(args)?;
    let rating: u8 = args
        .iter()
        .rev()
        .find_map(|a| a.parse::<u8>().ok().filter(|v| *v <= 5))
        .ok_or_else(|| gpp_core::Error::other("usage: gpp rate <id[,id...]> <0-5>"))?;

    let n = lib.set_rating_bulk(&ids, rating)?;
    println!("rated {n} photo(s) {rating}★");
    Ok(())
}

fn cmd_flag(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let ids = parse_ids(args)?;
    let flag = args
        .iter()
        .find_map(|a| match a.as_str() {
            "pick" => Some(Flag::Pick),
            "reject" => Some(Flag::Reject),
            "none" => Some(Flag::None),
            _ => None,
        })
        .ok_or_else(|| gpp_core::Error::other("usage: gpp flag <id[,id...]> <pick|reject|none>"))?;

    let n = lib.set_flag_bulk(&ids, flag)?;
    println!("flagged {n} photo(s) as {}", flag.as_str());
    Ok(())
}

fn parse_ids(args: &[String]) -> Result<Vec<i64>> {
    let raw = positional(args)
        .ok_or_else(|| gpp_core::Error::other("expected one or more photo ids"))?;
    let ids: Vec<i64> = raw.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if ids.is_empty() {
        return Err(gpp_core::Error::other("no valid photo ids given"));
    }
    Ok(ids)
}

/// `gpp develop <ids> [adjustments]` — non-destructive adjustments.
///
/// Every value is an upsert: passing `--exposure 0` clears exposure rather than
/// storing a zero, which is what puts the photo back on its original render key.
fn cmd_develop(args: &[String]) -> Result<()> {
    let session = open_session(args)?;
    let ids = parse_ids(args)?;

    if has(args, "--show") {
        for id in &ids {
            let stack = session.photo_edits(*id)?;
            if stack.is_empty() {
                println!("{id}: no adjustments");
                continue;
            }
            println!("{id}:");
            for op in &stack.ops {
                println!("  {}", describe_op(op));
            }
        }
        return Ok(());
    }

    if has(args, "--reset") {
        let n = session.reset_photo_edits(ids)?;
        println!("reset {n} photo(s) to the original");
        return Ok(());
    }

    let mut ops: Vec<EditOp> = Vec::new();
    let amount = |name: &str| opt(args, name).and_then(|v| v.parse::<f32>().ok());

    if let Some(ev) = amount("--exposure") { ops.push(EditOp::Exposure { ev }); }
    if let Some(a) = amount("--contrast") { ops.push(EditOp::Contrast { amount: a }); }
    if let Some(a) = amount("--saturation") { ops.push(EditOp::Saturation { amount: a }); }
    if let Some(a) = amount("--temperature") { ops.push(EditOp::Temperature { amount: a }); }
    if let Some(a) = amount("--tint") { ops.push(EditOp::Tint { amount: a }); }
    if let Some(a) = amount("--highlights") { ops.push(EditOp::Highlights { amount: a }); }
    if let Some(a) = amount("--shadows") { ops.push(EditOp::Shadows { amount: a }); }
    if let Some(q) = opt(args, "--rotate").and_then(|v| v.parse::<u8>().ok()) {
        ops.push(EditOp::Rotate { quarter_turns: q });
    }
    if let Some(spec) = opt(args, "--crop") {
        let n: Vec<f32> = spec.split(',').filter_map(|v| v.trim().parse().ok()).collect();
        if n.len() != 4 {
            return Err(gpp_core::Error::other("--crop wants x,y,w,h as fractions, e.g. 0.1,0,0.8,1"));
        }
        ops.push(EditOp::Crop { x: n[0], y: n[1], w: n[2], h: n[3] });
    }
    if has(args, "--bw") { ops.push(EditOp::BlackAndWhite); }
    if has(args, "--flip-h") { ops.push(EditOp::FlipHorizontal); }
    if has(args, "--flip-v") { ops.push(EditOp::FlipVertical); }

    let mut clears: Vec<&str> = Vec::new();
    if has(args, "--no-bw") { clears.push("black-and-white"); }

    if ops.is_empty() && clears.is_empty() {
        return Err(gpp_core::Error::other(
            "nothing to do — pass an adjustment, --show or --reset (see `gpp help`)",
        ));
    }

    let mut touched = 0;
    for op in ops {
        touched = touched.max(session.set_photo_edit(ids.clone(), op)?);
    }
    for kind in clears {
        touched = touched.max(session.clear_photo_edit(ids.clone(), kind.to_string())?);
    }
    println!("adjusted {touched} photo(s)");
    Ok(())
}

fn describe_op(op: &EditOp) -> String {
    match *op {
        EditOp::Exposure { ev } => format!("exposure     {ev:+.2} EV"),
        EditOp::Contrast { amount } => format!("contrast     {amount:+.0}"),
        EditOp::Saturation { amount } => format!("saturation   {amount:+.0}"),
        EditOp::Temperature { amount } => format!("temperature  {amount:+.0}"),
        EditOp::Tint { amount } => format!("tint         {amount:+.0}"),
        EditOp::Highlights { amount } => format!("highlights   {amount:+.0}"),
        EditOp::Shadows { amount } => format!("shadows      {amount:+.0}"),
        EditOp::BlackAndWhite => "black & white".to_string(),
        EditOp::Rotate { quarter_turns } => format!("rotate       {}°", quarter_turns as u32 * 90),
        EditOp::FlipHorizontal => "flip         horizontal".to_string(),
        EditOp::FlipVertical => "flip         vertical".to_string(),
        EditOp::Crop { x, y, w, h } => format!("crop         {x:.3},{y:.3} {w:.3}×{h:.3}"),
    }
}

/// A session over the same library the other commands open.
///
/// Develop goes through `Session` rather than `Library` because rendering the
/// new thumbnails after an edit is part of the operation, and that lives there.
fn open_session(args: &[String]) -> Result<gpp_core::Session> {
    let session = gpp_core::Session::new();
    session.open_library(library_root(args)?)?;
    Ok(session)
}

fn cmd_album(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let sub = args
        .first()
        .map(|s| s.as_str())
        .ok_or_else(|| gpp_core::Error::other("usage: gpp album <list|create|show|add|set|move|rm>"))?;
    let rest = &args[1..];

    match sub {
        "list" => {
            for a in lib.albums()? {
                let lock = if a.is_locked() { " 🔒" } else { "" };
                let count = lib.album_photos(&a.path)?.len();
                println!("{:<40} {:>4} photos  {}{}", a.path, count, a.title, lock);
            }
        }
        "create" => {
            let path = positional(rest)
                .ok_or_else(|| gpp_core::Error::other("usage: gpp album create <path>"))?;
            let album = lib.create_album(&NewAlbum {
                path: path.to_string(),
                title: opt(rest, "--title").map(String::from),
                description: opt(rest, "--description").map(String::from),
                date: opt(rest, "--date").map(String::from),
                is_collection: has(rest, "--collection"),
            })?;
            println!("created {} ({})", album.path, album.title);
        }
        "show" => {
            let path = positional(rest)
                .ok_or_else(|| gpp_core::Error::other("usage: gpp album show <path>"))?;
            let album = lib
                .album_by_path(path)?
                .ok_or_else(|| gpp_core::Error::AlbumNotFound(path.into()))?;
            println!("{}", serde_json::to_string_pretty(&album)?);
            for p in lib.album_photos(path)? {
                println!("  {:>6}  {}", p.id, p.filename);
            }
        }
        "add" => {
            let path = positional(rest)
                .ok_or_else(|| gpp_core::Error::other("usage: gpp album add <path> --ids 1,2,3"))?;
            let ids: Vec<i64> = opt(rest, "--ids")
                .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
                .unwrap_or_default();
            let n = lib.add_photos_to_album(path, &ids)?;
            println!("added {n} photo(s) to {path}");
        }
        "set" => {
            let path = positional(rest)
                .ok_or_else(|| gpp_core::Error::other("usage: gpp album set <path> [--key value]"))?;
            let mut update = AlbumUpdate {
                title: opt(rest, "--title").map(String::from),
                sort: opt(rest, "--sort").map(String::from),
                style: opt(rest, "--style").map(String::from),
                ..Default::default()
            };
            if let Some(v) = opt(rest, "--password") {
                update.password = Some((!v.is_empty()).then(|| v.to_string()));
            }
            if has(rest, "--share-link") {
                update.share_token = Some(Some(gpp_core::albums::generate_share_token()));
            }
            if has(rest, "--no-share-link") {
                update.share_token = Some(None);
            }
            if has(rest, "--proofing") {
                update.proofing = Some(true);
            }
            if has(rest, "--no-proofing") {
                update.proofing = Some(false);
            }
            if has(rest, "--allow-download") {
                update.allow_download = Some(true);
            }
            if let Some(tags) = opt(rest, "--tags") {
                update.tags = Some(tags.split(',').map(|t| t.trim().to_string()).collect());
            }

            let album = lib.update_album(path, &update)?;
            println!("updated {}", album.path);
            if let Some(token) = &album.share_token {
                println!("share link: /photos/{}?token={}", album.path, token);
            }
        }
        "move" => {
            let from = positional(rest)
                .ok_or_else(|| gpp_core::Error::other("usage: gpp album move <from> --to <to>"))?;
            let to = opt(rest, "--to")
                .ok_or_else(|| gpp_core::Error::other("--to is required"))?;
            let album = lib.move_album(from, to)?;
            println!("moved to {}", album.path);
        }
        "rm" => {
            let path = positional(rest)
                .ok_or_else(|| gpp_core::Error::other("usage: gpp album rm <path>"))?;
            lib.delete_album(path)?;
            println!("deleted album {path} (photos untouched)");
        }
        other => {
            return Err(gpp_core::Error::other(format!(
                "unknown album subcommand '{other}'"
            )))
        }
    }
    Ok(())
}

/// `gpp lr <scan|import> <catalog.lrcat>` — Lightroom Classic catalogs.
fn cmd_lr(args: &[String]) -> Result<()> {
    let sub = args
        .first()
        .map(|s| s.as_str())
        .ok_or_else(|| gpp_core::Error::other("usage: gpp lr <scan|import> <catalog.lrcat>"))?;
    let rest = &args[1..];
    let session = open_session(rest)?;
    let lrcat = positional(rest)
        .ok_or_else(|| gpp_core::Error::other("usage: gpp lr <scan|import> <catalog.lrcat>"))?
        .to_string();

    match sub {
        "scan" => {
            let report = session.lr_scan(lrcat)?;
            println!("catalog id: {}", report.catalog_id);
            println!("\nroot folders:");
            for r in &report.root_folders {
                println!(
                    "  {:<40} {:>5} files, {} missing — {}",
                    r.path,
                    r.file_count,
                    r.missing_files,
                    if r.in_place {
                        "inside a source (imports in place)".to_string()
                    } else if r.can_reference {
                        "outside every source (copies in, or --mode reference \
                         to catalogue it where it is)"
                            .to_string()
                    } else {
                        "outside every source, and not a readable folder".to_string()
                    }
                );
            }
            if !report.collections.is_empty() {
                println!("\ncollections:");
                for c in &report.collections {
                    println!("  {:<40} {:<10} {:>5} members", c.path, c.kind, c.member_count);
                }
            }
            println!("\nkeywords: {}", report.keyword_count);
            if report.images_without_files > 0 {
                println!("images without files: {}", report.images_without_files);
            }
            for note in &report.notes {
                eprintln!("note: {note}");
            }
        }
        "import" => {
            let options = gpp_core::lightroom::LrImportOptions {
                dest_subdir: opt(rest, "--dest-subdir").map(String::from),
                collections: opt(rest, "--collections")
                    .map(|v| v.split(',').map(|s| s.trim().to_string()).collect()),
                album_prefix: opt(rest, "--prefix").map(String::from),
                collision: match opt(rest, "--collision") {
                    Some("merge") => gpp_core::lightroom::MergePolicy::Merge,
                    Some("suffix") => gpp_core::lightroom::MergePolicy::Suffix,
                    Some(other) => {
                        return Err(gpp_core::Error::other(format!(
                            "--collision wants merge or suffix, not '{other}'"
                        )))
                    }
                    None => gpp_core::lightroom::MergePolicy::Auto,
                },
                mode: placement_from(rest)?,
                // Per-root placement is a UI affordance (tick a box per root
                // folder); on the command line `--mode` covers every root, and
                // a root-by-root list would be unreadable as a flag.
                roots: None,
                dry_run: has(rest, "--dry-run"),
            };

            let last = std::sync::Mutex::new(0usize);
            let report = session.lr_import(
                lrcat,
                options,
                Some(&|p| {
                    let mut last = last.lock().unwrap();
                    if p.processed == p.total || p.processed >= *last + 25 {
                        *last = p.processed;
                        eprintln!("  [{}/{}] {}", p.processed, p.total, p.current);
                    }
                }),
            )?;

            if report.dry_run {
                println!("dry run — nothing was written; this is what an import would do:");
            }
            println!(
                "photos: {} copied ({} bytes) · {} in place ({} referenced) · \
                 {} linked to existing · {} missing",
                report.photos_copied,
                report.bytes_copied,
                report.photos_in_place,
                report.photos_referenced,
                report.photos_linked_existing,
                report.skipped_missing_files
            );
            // Registering a source is a lasting change to the library — it is
            // what makes those files findable next time — so never let it pass
            // unremarked.
            for s in &report.sources_registered {
                println!("  registered as a source (nothing copied): {s}");
            }
            println!(
                "albums: {} created · {} updated · {} memberships · {} tag links",
                report.albums_created,
                report.albums_updated,
                report.memberships_added,
                report.tags_added
            );
            for c in &report.collisions {
                println!("  collision: {c}");
            }
            for c in &report.conflicts {
                println!("  conflict: {c}");
            }
            for d in &report.lr_deleted {
                println!("  deleted in Lightroom, kept here: {d}");
            }
            if report.cancelled {
                println!("stopped early — every count above is a partial tally");
            }
        }
        other => {
            return Err(gpp_core::Error::other(format!(
                "unknown lr subcommand '{other}' — scan or import"
            )))
        }
    }
    Ok(())
}

fn cmd_publish(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let dest = opt(args, "--dest").ok_or_else(|| {
        gpp_core::Error::other("--dest <gallery/src/content/albums> is required")
    })?;
    let dest = Path::new(dest);

    let opts = PublishOptions {
        min_rating: opt(args, "--min-rating").and_then(|v| v.parse().ok()),
        exclude_rejected: !has(args, "--include-rejected"),
        copy_photos: !has(args, "--metadata-only"),
    };

    // A specific album, or every album when none is named.
    let targets: Vec<String> = match positional(args) {
        Some(p) => vec![p.to_string()],
        None => lib.albums()?.into_iter().map(|a| a.path).collect(),
    };

    let mut copied = 0;
    for path in &targets {
        let r = publish_album(&lib, path, dest, &opts)?;
        copied += r.photos_copied;
        println!(
            "{:<40} {} copied, {} unchanged{}",
            r.album_path,
            r.photos_copied,
            r.photos_skipped,
            if r.missing.is_empty() {
                String::new()
            } else {
                format!(", {} missing on disk", r.missing.len())
            }
        );
        for m in &r.missing {
            println!("    missing: {m}");
        }

        // Not missing — on a drive that is not plugged in. The album is short
        // until it is, and the next publish ships them.
        for o in &r.offline {
            eprintln!("    not published, source unavailable: {o}");
        }

        // A frame the developer could not render is left out of the album, and
        // a publish that says nothing about it looks exactly like a clean one —
        // the photographer finds out from a client asking where the photo went.
        for u in &r.unrenderable {
            eprintln!("    could not be rendered, not published: {u}");
        }

        // Two frames wanting one published name. Naming both sides is the
        // whole point: only the photographer can say which one the client
        // should get, and the other is not on the site until they do.
        for c in &r.collisions {
            eprintln!("    name taken: {} — published {}", c.dest, c.sources[0]);
            for skipped in &c.sources[1..] {
                eprintln!("      not published: {skipped}");
            }
        }
    }
    println!("published {} album(s), {} file(s) copied", targets.len(), copied);
    Ok(())
}

fn cmd_sync(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let root = published_root_for(&lib, args)?;
    let direction = direction_from(args);
    let allow_deletes = has(args, "--allow-deletes");
    // `--scope full` re-tracks the named album with full scope before syncing.
    if let (Some(album), Some(scope)) = (positional(args), scope_from(args)?) {
        let id = lib.ensure_default_remote()?;
        lib.track_album_for(album, direction, Some(scope), id)?;
    }

    // With an album named, sync just that one. Without, sync everything this
    // machine has subscribed to — each album in its own direction.
    match positional(args) {
        Some(album) => {
            let transport = transport_for(&lib, args)?;
            if has(args, "--plan") {
                let plan = remote::plan_album_sync(&lib, transport.as_ref(), album, direction, &root)?;
                print_plan(&plan);
                return Ok(());
            }
            let opts = PublishOptions::default();
            let outcome = remote::sync_path(
                &lib, transport.as_ref(), album, direction, &root, &opts, allow_deletes,
            )?;
            println!(
                "{album}: pushed {} · pulled {} · deleted {} · skipped {}",
                outcome.pushed, outcome.pulled, outcome.deleted, outcome.skipped
            );
            for c in &outcome.conflicts {
                println!("  conflict: {c}");
            }
            for (path, why) in &outcome.failed {
                println!("  failed: {path} — {why}");
            }
        }
        None => {
            let transport = transport_for(&lib, args)?;
            let subs = lib.album_subscriptions()?;
            if subs.is_empty() {
                println!("No albums tracked. Track one with:  gpp sync <album> --pull");
                return Ok(());
            }
            let opts = PublishOptions::default();
            let results =
                remote::sync_tracked_albums(&lib, transport.as_ref(), &root, &opts, allow_deletes)?;
            for (album, outcome) in &results {
                println!(
                    "{album}: pushed {} · pulled {} · deleted {} · skipped {}",
                    outcome.pushed, outcome.pulled, outcome.deleted, outcome.skipped
                );
                for c in &outcome.conflicts {
                    println!("  conflict: {c}");
                }
                // A whole album can land here — a subscription naming an album
                // that was renamed or deleted, say. The batch carries on, so
                // this is the only place it is said out loud.
                for (path, why) in &outcome.failed {
                    println!("  failed: {path} — {why}");
                }
            }
        }
    }
    Ok(())
}

fn print_plan(plan: &sync::SyncPlan) {
    println!("push          {}", plan.count(sync::Action::Push));
    println!("pull          {}", plan.count(sync::Action::Pull));
    println!("delete remote {}", plan.count(sync::Action::DeleteRemote));
    println!("conflicts     {}", plan.count(sync::Action::Conflict));
    println!("left alone    {}", plan.count(sync::Action::LeaveAlone));
    println!("skipped       {}", plan.count(sync::Action::Skip));
    if plan.has_conflicts() {
        println!("\nconflicts:");
        for c in plan.of(sync::Action::Conflict) {
            println!("  {}", c.path);
        }
    }
    if plan.is_destructive() {
        println!("\nwould delete on the server (needs --allow-deletes):");
        for c in plan.of(sync::Action::DeleteRemote) {
            println!("  {}", c.path);
        }
    }
}

/// Persist an HTTP remote's access token onto the default remote, if given.
fn store_remote_token(args: &[String], lib: &Library) -> Result<()> {
    if let Some(token) = opt(args, "--remote-token") {
        let id = lib.ensure_default_remote()?;
        lib.update_remote(
            id,
            &RemoteUpdate { token: Some(Some(token.to_string())), ..Default::default() },
        )?;
    }
    Ok(())
}

/// Build a transport for a bare target/token pair — a path is a directory, an
/// `http(s)` URL is the sync API. Same rule the desktop app uses, so a library
/// configured by one is configured for the other.
fn transport_for_spec(
    target: &str,
    token: Option<&str>,
) -> Result<Box<dyn gpp_core::sync::RemoteTransport>> {
    if target.starts_with("http://") || target.starts_with("https://") {
        let token = token.filter(|t| !t.is_empty()).ok_or_else(|| {
            gpp_core::Error::other(
                "this remote needs an access token — pass --token (or --remote-token) once",
            )
        })?;
        return Ok(Box::new(gpp_core::sync::HttpTransport::new(target, token)));
    }
    Ok(Box::new(gpp_core::sync::FsTransport::new(target)))
}

/// The default remote's transport: `--remote <dir-or-url>` updates it in the
/// catalog first (remotes row #1 after migration), so the setting survives.
fn transport_for(
    lib: &Library,
    args: &[String],
) -> Result<Box<dyn gpp_core::sync::RemoteTransport>> {
    if let Some(d) = opt(args, "--remote") {
        let id = lib.ensure_default_remote()?;
        lib.update_remote(id, &RemoteUpdate { target: Some(d.to_string()), ..Default::default() })?;
    }
    store_remote_token(args, lib)?;

    let info = lib
        .default_remote_id()?
        .and_then(|id| lib.remote_by_id(id).transpose())
        .transpose()?
        .filter(|r| !r.target.is_empty())
        .ok_or_else(|| {
            gpp_core::Error::other("no remote set — pass --remote <dir-or-url> once")
        })?;
    transport_for_spec(&info.target, info.token.as_deref())
}

/// Chosen sync scope: `--scope web|full`, defaulting to web.
fn scope_from(args: &[String]) -> Result<Option<SyncScopeKind>> {
    match opt(args, "--scope") {
        None => Ok(None),
        Some("web") => Ok(Some(SyncScopeKind::Web)),
        Some("full") => Ok(Some(SyncScopeKind::Full)),
        Some(other) => Err(gpp_core::Error::other(format!(
            "--scope wants web or full, not '{other}'"
        ))),
    }
}

/// `gpp remotes` — manage this library's remotes, or list another library's.
fn cmd_remotes(args: &[String]) -> Result<()> {
    // Another library's remotes: read-only, no session swap, no migration.
    if let Some(root) = opt(args, "--of") {
        let listed = gpp_core::remotes::read_library_remotes(root)?;
        if listed.is_empty() {
            println!("No remotes configured in {root}.");
            return Ok(());
        }
        for r in &listed {
            println!(
                "{:>3}  {:<20} {}{}{}",
                r.id,
                r.name,
                r.target,
                if r.token.is_some() { "  [token]" } else { "" },
                if r.is_default { "  (default)" } else { "" },
            );
        }
        return Ok(());
    }

    let lib = open_library(args)?;
    match args.first().map(|s| s.as_str()) {
        Some("add") => {
            let target = opt(args, "--target")
                .or_else(|| positional(&args[1..]))
                .ok_or_else(|| {
                    gpp_core::Error::other("usage: gpp remotes add <dir-or-url> [--name N] [--token T]")
                })?;
            let name = opt(args, "--name").unwrap_or("Remote");
            let id = lib.add_remote(name, target, opt(args, "--token"))?;
            println!("added remote #{id} ({name}) -> {target}");
        }
        Some("rm") => {
            let id: i64 = positional(&args[1..])
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| gpp_core::Error::other("usage: gpp remotes rm <id>"))?;
            lib.remove_remote(id)?;
            println!("removed remote #{id} (its subscriptions and baselines here; the server is untouched)");
        }
        Some("default") => {
            let id: i64 = positional(&args[1..])
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| gpp_core::Error::other("usage: gpp remotes default <id>"))?;
            lib.set_default_remote(id)?;
            println!("default remote is now #{id}");
        }
        _ => {
            let listed = lib.remotes()?;
            if listed.is_empty() {
                println!("No remotes yet. Add one with:  gpp remotes add <dir-or-url> --name N");
                return Ok(());
            }
            for r in &listed {
                let subs = lib.album_subscriptions_for(r.id)?.len();
                println!(
                    "{:>3}  {:<20} {:<40} {:>3} tracked{}{}",
                    r.id,
                    r.name,
                    if r.target.is_empty() { "(no target set)" } else { &r.target },
                    subs,
                    if r.token.is_some() { "  [token]" } else { "" },
                    if r.is_default { "  (default)" } else { "" },
                );
            }
        }
    }
    Ok(())
}

/// Chosen Lightroom placement: `--mode auto|in-place|copy|reference`.
fn placement_from(args: &[String]) -> Result<gpp_core::lightroom::LrPlacement> {
    use gpp_core::lightroom::LrPlacement;
    match opt(args, "--mode") {
        None | Some("auto") => Ok(LrPlacement::Auto),
        Some("in-place") => Ok(LrPlacement::InPlace),
        Some("copy") => Ok(LrPlacement::Copy),
        Some("reference") => Ok(LrPlacement::Reference),
        Some(other) => Err(gpp_core::Error::other(format!(
            "--mode wants auto, in-place, copy or reference, not '{other}'"
        ))),
    }
}

/// `gpp sources` — the roots photographs may live under.
///
/// Registering one is what makes "import without moving my files" possible:
/// afterwards an import of that folder catalogues it where it lies. Nothing
/// here ever writes to, moves or deletes a photograph.
fn cmd_sources(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let rest = if args.is_empty() { args } else { &args[1..] };

    match args.first().map(|s| s.as_str()) {
        Some("add") => {
            let path = opt(args, "--path")
                .or_else(|| positional(rest))
                .ok_or_else(|| {
                    gpp_core::Error::other(
                        "usage: gpp sources add <folder> [--name N] [--kind internal|external|network]",
                    )
                })?;
            let kind = match opt(args, "--kind") {
                None => None,
                Some("internal") => Some(gpp_core::SourceKind::Internal),
                Some("external") => Some(gpp_core::SourceKind::External),
                Some("network") => Some(gpp_core::SourceKind::Network),
                Some(other) => {
                    return Err(gpp_core::Error::other(format!(
                        "--kind wants internal, external or network, not '{other}'"
                    )))
                }
            };
            let id = lib.add_source(Path::new(path), opt(args, "--name"), kind)?;
            println!("added source #{id} -> {path}");
            println!("nothing was catalogued — run:  gpp import {path}");
        }
        Some("rm") => {
            let id: i64 = positional(rest)
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| {
                    gpp_core::Error::other("usage: gpp sources rm <id> [--drop-photos]")
                })?;
            let dropped = lib.remove_source(id, has(args, "--drop-photos"))?;
            println!("removed source #{id} — {dropped} catalog row(s) dropped, no files touched");
        }
        Some("relocate") => {
            let id: i64 = positional(rest)
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| {
                    gpp_core::Error::other("usage: gpp sources relocate <id> --to <folder>")
                })?;
            let to = opt(args, "--to")
                .ok_or_else(|| gpp_core::Error::other("--to <folder> is required"))?;
            lib.relocate_source(id, Path::new(to))?;
            println!("source #{id} now at {to}");
        }
        _ => {
            for s in lib.sources()? {
                println!(
                    "{:>3}  {:<20} {:<10} {:>6} photos  {:<8} {}",
                    s.id,
                    s.name,
                    if s.is_primary { "primary" } else { "referenced" },
                    s.photo_count,
                    if s.online { "online" } else { "OFFLINE" },
                    s.path,
                );
            }
            println!("\nAdd one with:  gpp sources add <folder> --name N");
        }
    }
    Ok(())
}

/// `gpp push-to` — stateless push to an arbitrary remote (typically another
/// library's, listed with `gpp remotes --of <root>`). Never deletes; names
/// every overwrite.
fn cmd_push_to(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let root = published_root_for(&lib, args)?;
    let album = positional(args).ok_or_else(|| {
        gpp_core::Error::other(
            "usage: gpp push-to --target <url|dir> [--token T] [--scope web|full] <album>",
        )
    })?;
    let target = opt(args, "--target")
        .ok_or_else(|| gpp_core::Error::other("--target <url|dir> is required"))?;
    let transport = transport_for_spec(target, opt(args, "--token"))?;

    let opts = PublishOptions {
        min_rating: opt(args, "--min-rating").and_then(|v| v.parse().ok()),
        ..Default::default()
    };
    let outcome = remote::push_album_to(
        &lib,
        transport.as_ref(),
        album,
        &root,
        &opts,
        scope_from(args)?.unwrap_or_default(),
    )?;
    println!(
        "pushed {} file(s) from {} album(s) to {target} (as library {} '{}'), {} unchanged",
        outcome.files_pushed,
        outcome.albums.len(),
        outcome.library_id,
        outcome.library_name,
        outcome.skipped_unchanged
    );
    if !outcome.overwritten.is_empty() {
        println!(
            "{} file(s) on that remote held a DIFFERENT version and were overwritten:",
            outcome.overwritten.len()
        );
        for p in &outcome.overwritten {
            println!("  {p}");
        }
    }
    if !outcome.failed.is_empty() {
        println!("{} file(s) did NOT reach the remote:", outcome.failed.len());
        for (path, why) in &outcome.failed {
            println!("  {path}: {why}");
        }
    }
    Ok(())
}

/// `gpp xmp export [album]` — write XMP sidecars beside the originals.
fn cmd_xmp(args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str());
    if sub != Some("export") {
        return Err(gpp_core::Error::other("usage: gpp xmp export [album]"));
    }
    let rest = &args[1..];
    let lib = open_library(rest)?;
    let outcome = lib.export_xmp(positional(rest))?;
    println!("wrote {} sidecar(s)", outcome.written);
    if !outcome.skipped_foreign.is_empty() {
        println!(
            "{} sidecar(s) from another tool were left alone:",
            outcome.skipped_foreign.len()
        );
        for p in &outcome.skipped_foreign {
            println!("  {p}");
        }
    }
    for m in &outcome.missing {
        println!("  original missing, no sidecar written: {m}");
    }
    for o in &outcome.offline {
        println!("  source unavailable, no sidecar written: {o}");
    }
    Ok(())
}

fn published_root_for(lib: &Library, args: &[String]) -> Result<PathBuf> {
    if let Some(d) = opt(args, "--dest") {
        let id = lib.ensure_default_target()?;
        lib.update_publish_target(
            id,
            &gpp_core::remotes::PublishTargetUpdate {
                dest_root: Some(d.to_string()),
                ..Default::default()
            },
        )?;
        return Ok(PathBuf::from(d));
    }
    lib.default_target_id()?
        .and_then(|id| lib.publish_target_by_id(id).transpose())
        .transpose()?
        .map(|t| t.dest_root)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| gpp_core::Error::other("no publish destination — pass --dest <dir> once"))
}

fn direction_from(args: &[String]) -> SyncDirection {
    if has(args, "--push") {
        SyncDirection::Push
    } else if has(args, "--pull") {
        SyncDirection::Pull
    } else {
        SyncDirection::Both
    }
}

/// `gpp remote` — what the server has, and what this machine tracks.
fn cmd_remote(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let transport = transport_for(&lib, args)?;

    let albums = remote::remote_albums(&lib, transport.as_ref())?;
    if albums.is_empty() {
        println!("No albums here or on the remote yet.");
        return Ok(());
    }

    println!("{:<36} {:>5}  {:<7} {:<7} SYNC", "ALBUM", "FILES", "LOCAL", "REMOTE");
    for a in &albums {
        println!(
            "{:<36} {:>5}  {:<7} {:<7} {}",
            a.path,
            a.file_count,
            if a.local { "yes" } else { "—" },
            if a.remote { "yes" } else { "—" },
            a.tracked.map(|d| d.as_str()).unwrap_or("not tracked")
        );
    }
    println!("\nAdopt one with:  gpp pull <album>");
    println!("Contribute one:  gpp push <album>");
    Ok(())
}

/// `gpp pull <path>` — adopt a path from the remote, folders and all.
fn cmd_pull(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let transport = transport_for(&lib, args)?;
    let root = published_root_for(&lib, args)?;
    let album = positional(args)
        .ok_or_else(|| gpp_core::Error::other("usage: gpp pull <path> [--remote D] [--dest D]"))?;

    let outcome = remote::pull_path(&lib, transport.as_ref(), album, &root)?;
    println!(
        "pulled {} file(s) into {} album(s), catalogued {} photo(s)",
        outcome.files_pulled,
        outcome.albums.len(),
        outcome.photos_imported
    );
    for a in &outcome.albums {
        println!("  {a}");
    }
    if !outcome.conflicts.is_empty() {
        println!("conflicts (nothing overwritten):");
        for c in &outcome.conflicts {
            println!("  {c}");
        }
    }
    // The gallery copy took the server's version; the negative in the library
    // did not, and there is no other copy of it.
    if !outcome.kept_originals.is_empty() {
        println!("originals kept (the server has a different version):");
        for c in &outcome.kept_originals {
            println!("  {c}");
        }
    }
    // An honest server never offers one of these, so a line here is worth
    // reading even though the album itself arrived.
    if !outcome.rejected.is_empty() {
        println!("refused (the server named files this machine will not write):");
        for c in &outcome.rejected {
            println!("  {c}");
        }
    }
    Ok(())
}

/// `gpp push <path>` — publish a path and upload it, folders and all.
fn cmd_push(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let transport = transport_for(&lib, args)?;
    let root = published_root_for(&lib, args)?;
    let album = positional(args)
        .ok_or_else(|| gpp_core::Error::other("usage: gpp push <path> [--allow-deletes]"))?;

    let opts = PublishOptions {
        min_rating: opt(args, "--min-rating").and_then(|v| v.parse().ok()),
        ..Default::default()
    };
    let outcome = remote::push_path(
        &lib, transport.as_ref(), album, &root, &opts, has(args, "--allow-deletes"),
    )?;
    println!(
        "pushed {} file(s) from {} album(s), deleted {} remotely, skipped {}",
        outcome.files_pushed,
        outcome.albums.len(),
        outcome.deleted_remote,
        outcome.skipped
    );
    for a in &outcome.albums {
        println!("  {a}");
    }
    if !outcome.folders_left_alone.is_empty() {
        println!(
            "{} parent folder(s) already on the server, configured elsewhere — left alone:",
            outcome.folders_left_alone.len()
        );
        for f in &outcome.folders_left_alone {
            println!("  {f}");
        }
        println!("push a folder directly to change it: gpp push <folder>");
    }
    if !outcome.withheld_deletes.is_empty() {
        println!(
            "{} file(s) on the server that this path no longer has:",
            outcome.withheld_deletes.len()
        );
        for p in &outcome.withheld_deletes {
            println!("  {p}");
        }
        println!("re-run with --allow-deletes to remove them");
    }
    if !outcome.conflicts.is_empty() {
        println!("conflicts (nothing overwritten):");
        for c in &outcome.conflicts {
            println!("  {c}");
        }
    }
    // One file failing does not stop the transfer, so it has to be said out
    // loud here — otherwise the gallery is a photo short and nothing said so.
    if !outcome.failed.is_empty() {
        println!("{} file(s) did NOT reach the server:", outcome.failed.len());
        for (path, why) in &outcome.failed {
            println!("  {path}: {why}");
        }
        println!("re-run to try them again");
    }
    Ok(())
}

fn cmd_stats(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    println!("library : {}", lib.root().display());
    println!("photos  : {}", lib.photo_count()?);
    println!("albums  : {}", lib.albums()?.len());
    let cameras = lib.camera_models()?;
    if !cameras.is_empty() {
        println!("cameras : {}", cameras.join(", "));
    }
    Ok(())
}

fn print_usage() {
    println!(
        r#"gpp — Goldplated Photos

USAGE
  gpp <command> [--library <path>]        (or set GPP_LIBRARY)

COMMANDS
  init [path]                     Create/open a library
  import [dir] [--force]          Scan, hash, extract metadata, build thumbnails
                                  [--no-thumbs] [--no-recursive]
                                  A dir inside a registered source is catalogued
                                  where it lies; one outside every source is
                                  copied in. With no dir: every source in turn.

  sources                         Roots photographs may live under
  sources add <folder> [--name N] [--kind internal|external|network]
                                  Register a folder: its photographs are
                                  catalogued where they are, never copied
  sources rm <id> [--drop-photos] Forget a source (never deletes a file)
  sources relocate <id> --to <folder>
                                  The drive moved. Validated by re-hashing a
                                  few of that source's own files at the new path
  ls [filters]                    List photos
                                  [--min-rating N] [--flag pick|reject]
                                  [--album PATH] [--camera NAME] [--search TEXT]
                                  [--from DATE] [--to DATE] [--limit N]
                                  [--sort name|rating|newest|album] [--json]
  rate <ids> <0-5>                Set rating (ids comma-separated)
  flag <ids> <pick|reject|none>   Set flag
  stats                           Library summary

  develop <ids> [adjustments]     Non-destructive adjustments (originals untouched)
                                  [--exposure EV] [--contrast N] [--saturation N]
                                  [--temperature N] [--tint N]
                                  [--highlights N] [--shadows N]
                                  [--bw|--no-bw] [--rotate 0-3]
                                  [--flip-h] [--flip-v] [--crop x,y,w,h]
  develop <ids> --show            Show what is applied
  develop <ids> --reset           Back to the original

  album list                      All albums
  album create <path> [--title T] [--collection]
  album show <path>
  album add <path> --ids 1,2,3
  album set <path> [--title T] [--sort S] [--style S] [--password P]
                   [--share-link] [--no-share-link] [--proofing]
                   [--allow-download] [--tags a,b]
  album move <from> --to <to>
  album rm <path>

  lr scan <catalog.lrcat>         Look inside a Lightroom Classic catalog:
                                  root folders (copy vs in-place), collections,
                                  keywords, missing files. Writes nothing.
  lr import <catalog.lrcat>       Import it: collections -> albums, keywords ->
                                  tags. Idempotent: re-running syncs, never
                                  duplicates.
                                  [--mode auto|in-place|copy|reference]
                                    auto      (default) in place where the root
                                              already sits in a source, else copy
                                    reference register each outside root as a
                                              source and catalogue it where it
                                              is — nothing copied, nothing moved
                                    copy      always copy into the library
                                  [--dry-run] [--collections a,b/*] [--prefix p]
                                  [--dest-subdir d] [--collision merge|suffix]

  publish [album] --dest <dir>    Write the gallery content tree
                                  [--min-rating N] [--metadata-only]
                                  [--include-rejected]

  remote [--remote <dir>]         List albums on the remote and what you track
  remotes                         List this library's remotes (id, name, target)
  remotes add <dir-or-url> [--name N] [--token T]
  remotes rm <id>                 Forget a remote (local state only)
  remotes default <id>            Choose the default remote
  remotes --of <library-root>     List ANOTHER library's remotes (read-only)
  push-to --target <url|dir> [--token T] [--scope web|full] <album>
                                  Stateless push to an arbitrary remote:
                                  never deletes, names every overwrite
  xmp export [album]              Write XMP sidecars (rating, label, keywords,
                                  orientation, develop stack) beside originals
  pull <path>                     Adopt a path (album or folder) and everything
                                  under it, plus the folders above it
  push <path> [--allow-deletes]   Publish a path and upload it, folders and all
  sync [path] [--push|--pull|--both] [--plan] [--allow-deletes] [--scope web|full]
                                  Sync one path, or every tracked path;
                                  --scope full also moves originals + metadata

EXAMPLES
  gpp init ~/Photos
  gpp import ~/Photos/2026-ana
  gpp ls --min-rating 4 --sort rating
  gpp album create 2026/ana --title "Ana & Ivan"
  gpp album set 2026/ana --password tajna --share-link --proofing
  gpp publish 2026/ana --dest ../src/content/albums --min-rating 3

  # Two machines, one server (a share, a drive, or a synced folder)
  gpp push 2026/ana --remote /Volumes/gallery --dest ../src/content/albums
  gpp remote                            # on the other machine: what's there?
  gpp pull 2026/ana                     # adopt it, photos and settings
  gpp sync 2026/ana --both              # thereafter: reconcile both ways
"#
    );
}
