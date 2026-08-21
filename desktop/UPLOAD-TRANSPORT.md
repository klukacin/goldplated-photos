# Upload transport — how to get photos to the server without SSH

Today `deploy.sh` runs `rsync` over SSH. This records what to do if SSH is not
available, so the question does not have to be researched again.

Workload assumed throughout: **one album of 100–800 JPEGs at 2–8 MB, sometimes
RAW at 25–50 MB**, from a photographer's laptop to a web gallery.

---

## The short answer

**HTTP/1.1 `PUT`, 4–8 parallel connections, keep-alive pooled, preceded by one
manifest exchange.** Explicitly pin HTTP/1.1 — do not let ALPN negotiate h2.

If server-side routes are impossible (static-only hosting), use **S3-compatible
object storage over its S3 API**, single `PUT` per file, `ListObjectsV2` + ETag
as the manifest.

Do not use WebDAV, FTPS, HTTP/3 or tus. Reasons below, each specific.

---

## 1. The link dominates. Everything else is about not wasting it.

800 × 4 MB is ~3.2 GB.

| Upstream | Floor for the transfer |
|---|---|
| 20 Mbit/s (typical consumer) | ~21 min |
| 100 Mbit/s (symmetric fibre) | ~4.3 min |

No protocol choice moves that by more than a few percent. What protocol choice
*can* do is leave most of the line unused. There are three documented ways to do
that, and avoiding them is the whole game.

---

## 2. HTTP/2 uploads are flow-control capped by default

This is the most actionable finding, and it is verified from primary sources.

Apache `mod_http2` — [`docs/manual/mod/mod_http2.xml`](https://github.com/apache/httpd/blob/trunk/docs/manual/mod/mod_http2.xml):

```
H2WindowSize 65535        (default)
"sets the size of the window that is used for flow control from client to
 server and limits the amount of data the server has to buffer"
```

nginx has the same shape — [`ngx_http_v2_module.xml`](https://github.com/nginx/nginx.org/blob/main/xml/en/docs/http/ngx_http_v2_module.xml):
`http2_body_preread_size` default **64k**.

64 KB in flight per stream means per-stream throughput is capped at
`window ÷ RTT`:

| RTT | Ceiling per HTTP/2 stream |
|---|---|
| 20 ms | 3.3 MB/s (26 Mbit/s) |
| 40 ms | 1.6 MB/s (13 Mbit/s) |
| 100 ms | 0.6 MB/s (5 Mbit/s) |

So on 100 Mbit/s upstream at 40 ms RTT, **one HTTP/2 stream through a stock
Apache tops out near 13 Mbit/s** — about an eighth of the line.

Cloudflare hit exactly this in production and fixed it with BDP-based autotuning;
notably the *goal* of their fix was to reach HTTP/1.1 parity, not to beat it
([blog](https://blog.cloudflare.com/delivering-http-2-upload-speed-improvements/),
not independently verified here).

**HTTP/2's headline win — multiplexing many requests over one connection — is a
download optimisation.** For uploads it replaces N independent TCP congestion
windows with one, and adds a tighter application-layer window on top. On shared
hosting you cannot tune `H2WindowSize`, so pin HTTP/1.1.

---

## 3. HTTP/3 is not an option here, and its upload performance is unmeasured

Four independent blockers, all verified:

- **Apache has no HTTP/3 module.** `mod_h3` does not exist in Apache trunk;
  `howto/http2.xml` mentions QUIC zero times. If the host is cPanel/Apache,
  this is the end of it.
- **nginx's HTTP/3 is self-declared experimental** and ships with
  `quic_gso off` by default, supported only where Linux has `UDP_SEGMENT` —
  meaning a stock deployment runs *without* QUIC's biggest CPU optimisation
  ([`ngx_http_v3_module.xml`](https://github.com/nginx/nginx.org/blob/main/xml/en/docs/http/ngx_http_v3_module.xml)).
- **reqwest's HTTP/3 is experimental**, gated behind `--cfg reqwest_unstable`
  ([Cargo.toml](https://github.com/seanmonstar/reqwest/blob/master/Cargo.toml)).
- **On macOS, batched QUIC sends need private Apple APIs** —
  `quinn-udp`'s `fast-apple-datapath` feature is documented as
  "Support private Apple APIs to send multiple packets in a single syscall"
  ([Cargo.toml](https://github.com/quinn-rs/quinn/blob/main/quinn-udp/Cargo.toml)).
  macOS is our first target, so we would ship either private-API use or one
  syscall per datagram.

And the honest gap: **no published benchmark of QUIC in the *upload* direction
was found.** All the QUIC throughput literature measures server→client. Treat
HTTP/3 upload performance as *unmeasured*, not as *bad*.

Where QUIC genuinely wins is lossy and high-delay links — hotel Wi-Fi,
tethering. Worth revisiting if that becomes the common case for our users.

---

## 4. rsync's delta engine was never helping us

This is the reassuring part, and it is why leaving SSH costs nothing.

rsync has two mechanisms, and only one is relevant to photos:

| Mechanism | Value for a photo gallery |
|---|---|
| File-list diff — skip files matching by name/size/mtime | **This is the entire benefit** |
| Block-level delta — send only changed blocks of a changed file | **Zero.** A JPEG is new or unchanged; it is never byte-edited in place |

Compression is the same story: JPEG and RAW are already entropy-coded, so
`Content-Encoding: gzip` yields ~0% and burns CPU at both ends. SSH's AES-GCM
and TLS's AES-GCM cost the same.

**So the SSH baseline has no per-byte advantage over HTTPS.** What we would give
up is a convenient client, which we are writing anyway — and `sync.rs` already
computes the file-list diff, with BLAKE3 instead of mtime guessing.

---

## 5. What actually determines the speed

In order of magnitude:

1. **The upstream link.** See §1.
2. **Parallelism — worth up to ~5×.** A single TCP flow starts at a ~14 KB
   congestion window (RFC 6928) and ramps geometrically; N flows ramp
   independently and fill the pipe far sooner. Where the bottleneck queue is
   per-flow fair, N flows claim N shares. This is why every speed test uses
   parallel streams. rclone's default is `--transfers 4`; 4–8 is the useful
   range, above ~8 you mostly add loss.
3. **Connection reuse — ~5% at worst, ~0% with a pool.** A fresh TLS 1.3
   connection costs ~2 RTT ≈ 80 ms at 40 ms RTT. 800 of those serialized is
   ~64 s against a 21-minute transfer. This term *inverts* for small files: at
   50 KB per file the handshake would cost more than the data.
4. **Congestion control.** Only matters on lossy/high-RTT paths.
5. **Server-side per-request cost.** Visible only at high concurrency with many
   small files.

**Our current `sync::apply` uploads serially.** Given item 2, that is the single
largest win available in this codebase — larger than any protocol choice.

---

## 6. Multipart upload is irrelevant at our file sizes

rclone — the most battle-tested S3 client there is — sets
`--s3-upload-cutoff` and `--b2-upload-cutoff` to **200 MiB**
([s3.md](https://github.com/rclone/rclone/blob/master/docs/content/s3.md),
[b2.md](https://github.com/rclone/rclone/blob/master/docs/content/b2.md)).

Every file we handle is far below that, so all of them are single-`PUT`
territory. Multipart exists to beat the 5 GB single-PUT ceiling and to resume a
file too big to cheaply restart. **Parallelise across files, not within them.**

---

## 7. Why not the alternatives

### WebDAV — disqualified by ownership, not by speed

A WebDAV `PUT` *is* an HTTP `PUT`; there is no per-byte penalty (though no
credible modern benchmark of `mod_dav` vs plain POST was found either way). Its
overhead is round-trips — `MKCOL`, `PROPFIND`, optional `LOCK`.

The disqualifier is [`mod_dav.xml`](https://github.com/apache/httpd/blob/trunk/docs/manual/mod/mod_dav.xml):

> "New files created will also be owned by this User and Group."
> "**The DAV repository is considered private to Apache; modifying files outside
> of Apache (for example using FTP or filesystem-level tools) should not be
> allowed.**"

`src/content/albums/**` has three non-Apache writers: the admin panel, the
Node/PM2 process, and rsync. Handing Apache exclusive ownership would make the
permission problems already documented in `CLAUDE.md` worse, not better.

Two further strikes: generic `mod_dav`/`cpdavd` supports **neither hashes nor
modification times**, so sync degrades to size-only comparison
([rclone webdav.md](https://github.com/rclone/rclone/blob/master/docs/content/webdav.md));
and availability on shared hosting is genuinely uncertain — sources contradict
each other on whether cPanel ships `mod_dav` or its own `cpdavd`, and some hosts
restrict it to VPS. **Must be confirmed per host.**

### FTPS — disqualified by no checksums

The per-file cost is real but small: FTP opens a new data connection per
transfer, and under FTPS each needs its own TLS handshake. At 40 ms RTT that is
~80 ms per file; 800 files across 6 parallel transfers is ~11 s, under 1% of the
transfer. Modern clients pool control connections and cache TLS sessions
(rclone's `--ftp-tls-cache-size`, default 32). **Anyone claiming FTPS is
unusably slow for 4 MB JPEGs is wrong** — the overhead only dominates for
tens-of-KB files.

The actual problem, from [rclone's ftp.md](https://github.com/rclone/rclone/blob/master/docs/content/ftp.md):

> "Rclone's FTP backend does not support any checksums but can compare file
> sizes."

Sync degrades to size-only: two different JPEGs of identical byte length would
be treated as the same file. Modification times are server-dependent. Add
passive-mode firewall grief, and plaintext FTP additionally sends client photos
and credentials in the clear.

### tus — solves the wrong resume problem

Well-engineered, seriously backed (editors from Transloadit, Apple and
Cloudflare), and heading for RFC status as
[draft-ietf-httpbis-resumable-upload](https://datatracker.ietf.org/doc/draft-ietf-httpbis-resumable-upload/),
currently at -12. With the `creation-with-upload` extension it costs one request
per file, the same as a plain `PUT`.

But what it buys is **byte-level resume within one file**. On a failed 4 MB JPEG
that saves re-sending 4 MB — a couple of seconds. The resume question that
actually matters is *"which of the 800 did we finish?"*, which is file-level, and
which the manifest exchange answers anyway.

Worth it if the unit of upload were a multi-GB video. It is not.

---

## 8. What to build

The seam already exists. `sync::RemoteTransport` is
`manifest / get / put / delete`, and `manifest()` returning the whole thing in
one call is exactly the right shape for one HTTP request.

**`HttpTransport`, against two new Astro routes:**

```
POST /api/sync/manifest   → client sends [{path, size, blake3}]
                            server replies with the subset it does not have
PUT  /api/sync/file?path= → one request per missing file
```

Both authenticated with the HMAC machinery already in `src/lib/access.ts`, with
the server verifying the declared BLAKE3 on receipt. One round trip decides the
whole album; the rest is parallel `PUT`s on a pooled connection.

**Make `sync::apply` parallel first.** Per §5 it is worth more than the transport
choice, it benefits `FsTransport` too, and it is a change inside one function.

**If server routes are impossible:** S3-compatible storage, single `PUT`,
`ListObjectsV2` + ETag/MD5 as the manifest — which is rsync's file-list phase
with a cryptographic comparison instead of an mtime heuristic. Prefer **R2** for
zero egress if the gallery is served from it (its
[Local Uploads](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/changelog/r2/2026-02-03-r2-local-uploads.mdx)
beta reports up to 75% TTLB reduction, free — but it is incompatible with
jurisdictional restrictions, which matters for an EU-locked bucket). Prefer
**B2 via its S3 endpoint** if raw storage cost dominates — never its native API,
which needs [two requests per file](https://github.com/rclone/rclone/blob/master/docs/content/b2.md).

Note this is an architectural change, not just a transport swap: access control
currently gates `/albums/*` through Node.

---

## Open questions — do not fill these with plausible numbers

1. **No upload-direction QUIC benchmark was found.** HTTP/3 upload performance
   is unmeasured, not proven bad.
2. **No modern `mod_dav` PUT vs plain POST benchmark was found.** The
   "no per-byte penalty" claim is reasoning from Apache's filter architecture.
3. **FTPS handshake cost:** only vendor-blog figures (100–300 ms). The
   percentages in §7 are arithmetic from first principles, not measurement.
4. **cPanel WebDAV availability: sources actively contradict each other.**
   Confirm with the specific host.
5. **AWS S3 and Backblaze pricing could not be confirmed from vendor pages**
   (egress-blocked during research). The R2 figures are primary-source.
6. **Cloudflare's ~100 MB proxied request-body limit on Free/Pro is
   search-summary only.** It would constrain 50 MB RAWs less than it looks, but
   verify before designing around it — it applies to anything behind an
   orange-clouded hostname, including our own Node endpoint.
7. A search summary claimed **rclone does rsync-style delta transfers. It does
   not** — zero occurrences of "delta" in its docs. Recorded because it is the
   kind of confident-and-wrong claim that ends up in a decision.
