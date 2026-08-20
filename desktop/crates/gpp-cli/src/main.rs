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
use gpp_core::import::{import_dir, ImportOptions};
use gpp_core::model::{Flag, PhotoFilter, PhotoSort};
use gpp_core::publish::{publish_album, PublishOptions};
use gpp_core::{sync, Library, Result};

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
        "publish" => cmd_publish(rest),
        "sync" => cmd_sync(rest),
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
    )?;

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
            "{:<40} {} copied, {} unchanged",
            r.album_path, r.photos_copied, r.photos_skipped
        );
    }
    println!("published {} album(s), {} file(s) copied", targets.len(), copied);
    Ok(())
}

fn cmd_sync(args: &[String]) -> Result<()> {
    let lib = open_library(args)?;
    let dest = opt(args, "--dest")
        .ok_or_else(|| gpp_core::Error::other("--dest <published tree> is required"))?;
    let dest = Path::new(dest);

    let local = gpp_core::publish::manifest_of(dest)?;
    let synced = lib.synced_manifest()?;

    // Without a configured transport we can still show what the local side
    // believes; a real remote manifest arrives once a transport is wired up.
    let remote = synced.clone();
    let scope = match opt(args, "--scope") {
        Some(s) => sync::SyncScope::with(s.split(',').map(|p| p.trim().to_string()).collect()),
        None => sync::SyncScope::everything(),
    };

    let p = sync::plan(&scope.filter(&local), &scope.filter(&synced), &scope.filter(&remote));

    println!("push          {}", p.count(sync::Action::Push));
    println!("pull          {}", p.count(sync::Action::Pull));
    println!("delete remote {}", p.count(sync::Action::DeleteRemote));
    println!("conflicts     {}", p.count(sync::Action::Conflict));
    println!("left alone    {}", p.count(sync::Action::LeaveAlone));

    if p.has_conflicts() {
        println!("\nconflicts need a decision:");
        for c in p.of(sync::Action::Conflict) {
            println!("  {}", c.path);
        }
    }
    if p.is_destructive() {
        println!("\nwould delete on the server (requires --allow-deletes):");
        for c in p.of(sync::Action::DeleteRemote) {
            println!("  {}", c.path);
        }
    }

    if has(args, "--record") {
        lib.record_synced_all(&local)?;
        println!("\nrecorded {} path(s) as synced", local.len());
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
  ls [filters]                    List photos
                                  [--min-rating N] [--flag pick|reject]
                                  [--album PATH] [--camera NAME] [--search TEXT]
                                  [--from DATE] [--to DATE] [--limit N]
                                  [--sort name|rating|newest|album] [--json]
  rate <ids> <0-5>                Set rating (ids comma-separated)
  flag <ids> <pick|reject|none>   Set flag
  stats                           Library summary

  album list                      All albums
  album create <path> [--title T] [--collection]
  album show <path>
  album add <path> --ids 1,2,3
  album set <path> [--title T] [--sort S] [--style S] [--password P]
                   [--share-link] [--no-share-link] [--proofing]
                   [--allow-download] [--tags a,b]
  album move <from> --to <to>
  album rm <path>

  publish [album] --dest <dir>    Write the gallery content tree
                                  [--min-rating N] [--metadata-only]
                                  [--include-rejected]
  sync --dest <dir> [--scope a,b] Show the sync plan [--record]

EXAMPLES
  gpp init ~/Photos
  gpp import ~/Photos/2026-ana
  gpp ls --min-rating 4 --sort rating
  gpp album create 2026/ana --title "Ana & Ivan"
  gpp album set 2026/ana --password tajna --share-link --proofing
  gpp publish 2026/ana --dest ../src/content/albums --min-rating 3
"#
    );
}
