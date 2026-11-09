//! ЧАСТЬ III (M4.0 (а), раунд 4): build the frozen world template **v3**.
//!
//! The official vanilla server generates a ~61×61 chunk area around the v2
//! ready-zone centre, using nothing but its own standard means — `forceload`
//! console commands, no source or jar change. The server runs on a fresh
//! copy of the frozen v2 template (`Vanilla::start` copies it every run),
//! v1/v2 stay untouched, and the busy work happens on that copy.
//!
//! Two phases on two separate vanilla starts:
//!
//! * **probe** — find the per-command chunk limit of `forceload add` by
//!   experiment (the owner's note says «~256, hypothesis», so it is measured,
//!   not assumed): a 1×N strip far outside the world is offered for N up to
//!   [`PROBE_HIGH`], then the largest accepted N is found by halving. The
//!   world of that run is discarded.
//! * **build** — forceload the target rectangle in strips the probe accepts,
//!   let vanilla generate (checked by `save-all flush` + a rescan of the
//!   `.mca`, never by elapsed time), then `forceload remove all`, verify the
//!   query is empty, and stop with `save-all flush` + `stop`. The copy is
//!   then moved to `C:\MineRust-data\world-template-v3`.
//!
//! Readiness is `Status=minecraft:full` in the region files, read by
//! `target/parity/chunk_status.py scan`; v3 is scanned and compared against
//! v2, and both the scan and the compare output are stored next to the
//! traces. The repository keeps only this source and the PROGRESS summary —
//! the world itself and the raw output stay outside the repository.
//!
//! ```text
//! cargo test -p parity --test build_template_v3 -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use parity::{copy_dir_all, load_parity_env, ParityEnv, Vanilla};

/// Where the report and scan output land (outside the repository).
const TRACE_DIR: &str = r"C:\MineRust-data\traces";
/// The artefact to produce.
const V3_DIR: &str = r"C:\MineRust-data\world-template-v3";
/// Centre of the v2 ready zone (spawn chunk); the 61×61 area is built around it.
const CENTRE: (i32, i32) = (6, -2);
/// Half-width of the target area: 2·30+1 = 61 chunks per axis.
const HALF: i32 = 30;
/// Upper bound of the per-command probe on a 1×N strip (`forceload add`).
const PROBE_HIGH: u64 = 8192;
/// Far-away chunk used by the limit probe (well outside any template).
const PROBE_CHUNK: (i32, i32) = (1000, 1000);
/// Pause between generation checks.
const GEN_PERIOD: Duration = Duration::from_secs(15);
/// Maximum `save-all`/rescan rounds before the run reports what exists.
const GEN_CHECKS: usize = 40;
/// How long one `sync` may wait for its `say` token: force-loading hundreds
/// of chunks makes single ticks long, and the token is printed after them.
const SYNC_TIMEOUT: Duration = Duration::from_secs(900);
/// Pause after each `forceload` command so its strip can generate.
const FORCELOAD_PACE: Duration = Duration::from_secs(2);

/// The target area as chunk coordinates: `(x0, x1, z0, z1)`, inclusive.
fn area() -> (i32, i32, i32, i32) {
    (
        CENTRE.0 - HALF,
        CENTRE.0 + HALF,
        CENTRE.1 - HALF,
        CENTRE.1 + HALF,
    )
}

/// The number of chunks the target area holds.
fn area_chunks() -> u64 {
    let side = 2 * HALF as u64 + 1;
    side * side
}

/// Block coordinates of a chunk rectangle: `(bx0, bz0, bx1, bz1)` inclusive.
fn blocks(x0: i32, x1: i32, z0: i32, z1: i32) -> (i32, i32, i32, i32) {
    (x0 * 16, z0 * 16, x1 * 16 + 15, z1 * 16 + 15)
}

/// The script that reads region `Status` (diagnostic helper outside the build).
fn chunk_status_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
        .join("chunk_status.py")
}

fn parity_env_or_skip() -> Option<ParityEnv> {
    match load_parity_env() {
        Ok(env) => Some(env),
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            None
        }
    }
}

/// Sends console commands and waits until the server has processed them.
///
/// Console commands run in order on the server thread, so a trailing
/// `say <token>` proves every earlier command finished; the token lands in
/// `latest.log` as a `System chat:` line. The returned string is the log
/// text written from just before the batch to the token.
struct Console<'a> {
    vanilla: &'a mut Vanilla,
    log: PathBuf,
    seq: u64,
}

impl Console<'_> {
    /// Runs `commands`, then `say <token>`, and returns the log delta.
    async fn sync(&mut self, commands: &[String]) -> Result<String, String> {
        self.seq += 1;
        let token = format!("MineRustPart3_{}", self.seq);
        let before = std::fs::read_to_string(&self.log)
            .map(|text| text.len())
            .unwrap_or(0);
        for command in commands {
            self.vanilla
                .command(command)
                .await
                .map_err(|error| error.to_string())?;
        }
        self.vanilla
            .command(&format!("say {token}"))
            .await
            .map_err(|error| error.to_string())?;
        let deadline = tokio::time::Instant::now() + SYNC_TIMEOUT;
        loop {
            if let Ok(text) = std::fs::read_to_string(&self.log) {
                if text.len() >= before {
                    let delta = &text[before..];
                    if delta.contains(&token) {
                        return Ok(delta.to_owned());
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("log never showed {token}"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

/// The notable log line of one `forceload` batch (the reply, not the echo).
fn notable(delta: &str) -> String {
    delta
        .lines()
        .map(str::trim)
        .find(|line| {
            line.contains("force loaded")
                || line.contains("Too many chunks")
                || line.contains("Unmarked")
        })
        .or_else(|| delta.lines().map(str::trim).find(|line| !line.is_empty()))
        .unwrap_or("")
        .to_owned()
}

/// True when a `forceload add` reply means the command was rejected.
fn rejected(delta: &str) -> bool {
    delta.contains("Too many chunks")
}

/// One probe attempt on a 1×N strip: `(accepted, reply line)`.
async fn probe(console: &mut Console<'_>, axis: u64) -> Result<(bool, String), String> {
    if axis == 0 {
        return Ok((true, "(zero chunks: trivially accepted)".to_owned()));
    }
    let (px, pz) = PROBE_CHUNK;
    let bx0 = px * 16;
    let bz0 = pz * 16;
    let bz1 = bz0 + axis as i32 * 16 - 1;
    let command = format!("forceload add {bx0} {bz0} {} {bz1}", bx0 + 15);
    let delta = console.sync(&[command]).await?;
    let accepted = !rejected(&delta);
    let reply = notable(&delta);
    console.sync(&["forceload remove all".to_owned()]).await?;
    Ok((accepted, reply))
}

/// Finds the largest N a single `forceload add` accepts, by halving.
async fn probe_limit(console: &mut Console<'_>) -> Result<(u64, Vec<String>), String> {
    let mut log = Vec::new();
    let (high_ok, reply) = probe(console, PROBE_HIGH).await?;
    log.push(format!(
        "probe {PROBE_HIGH} chunks: accepted={high_ok} | {reply}"
    ));
    if high_ok {
        return Ok((u64::MAX, log));
    }
    let (mut lo, mut hi) = (0u64, PROBE_HIGH);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        let (ok, reply) = probe(console, mid).await?;
        log.push(format!("probe {mid} chunks: accepted={ok} | {reply}"));
        if ok {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok((lo, log))
}

/// The result of one `chunk_status.py scan`.
#[derive(Default)]
struct Scan {
    /// Every chunk record found.
    total: usize,
    /// Records whose `Status` is `minecraft:full`.
    full: usize,
    /// `full` records inside the target area.
    full_in_area: usize,
    /// `(min_x, min_z, max_x, max_z)` of the `full` records — the ready zone.
    full_bbox: Option<(i32, i32, i32, i32)>,
    /// `Status` value → count, in first-seen order.
    statuses: Vec<(String, usize)>,
    /// `(min_x, min_z, max_x, max_z)` over every record.
    bbox: Option<(i32, i32, i32, i32)>,
}

impl Scan {
    /// Counts one status value.
    fn status(&self, name: &str) -> usize {
        self.statuses
            .iter()
            .find(|(key, _)| key == name)
            .map_or(0, |(_, count)| *count)
    }
}

/// Runs `chunk_status.py scan` and parses the `Status` rows it prints.
fn scan(world: &Path) -> Result<Scan, String> {
    let script = chunk_status_script();
    let output = Command::new("python")
        .arg(&script)
        .arg("scan")
        .arg(world)
        .output()
        .map_err(|error| format!("cannot run {}: {error}", script.display()))?;
    if !output.status.success() {
        return Err(format!(
            "chunk_status.py scan failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_scan(&text, area()))
}

/// Parses the `scan` output; each chunk line is `  (x, z) status=... secs=...`.
fn parse_scan(text: &str, area: (i32, i32, i32, i32)) -> Scan {
    let mut scan = Scan::default();
    let mut statuses: Vec<(String, usize)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('(') {
            continue;
        }
        let Some(close) = trimmed.find(')') else {
            continue;
        };
        let coords = &trimmed[1..close];
        let Some((x_raw, z_raw)) = coords.split_once(',') else {
            continue;
        };
        let (Ok(x), Ok(z)) = (x_raw.trim().parse::<i32>(), z_raw.trim().parse::<i32>()) else {
            continue;
        };
        let Some(status_at) = line.find("status=") else {
            continue;
        };
        let status: String = line[status_at + "status=".len()..]
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();
        scan.total += 1;
        if status == "minecraft:full" {
            scan.full += 1;
            if x >= area.0 && x <= area.1 && z >= area.2 && z <= area.3 {
                scan.full_in_area += 1;
            }
            scan.full_bbox = Some(match scan.full_bbox {
                None => (x, z, x, z),
                Some((x0, z0, x1, z1)) => (x0.min(x), z0.min(z), x1.max(x), z1.max(z)),
            });
        }
        match statuses.iter_mut().find(|(key, _)| *key == status) {
            Some((_, count)) => *count += 1,
            None => statuses.push((status, 1)),
        }
        scan.bbox = Some(match scan.bbox {
            None => (x, z, x, z),
            Some((x0, z0, x1, z1)) => (x0.min(x), z0.min(z), x1.max(x), z1.max(z)),
        });
    }
    scan.statuses = statuses;
    scan
}

/// Total size in bytes of every regular file under `dir`.
fn dir_bytes(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total += dir_bytes(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

/// Formats one byte count as `N bytes (M.M MB)`.
fn mb(bytes: u64) -> String {
    format!("{bytes} B ({:.1} MB)", bytes as f64 / 1_048_576.0)
}

/// Disables vanilla's watchdog (`max-tick-time=-1`) while the build runs and
/// restores `server.properties` verbatim on drop.
///
/// Force-loading hundreds of chunks makes single ticks long; the default 60 s
/// watchdog force-shut the server down on the first attempt. `max-tick-time`
/// is the standard server property for exactly that, and this guard touches
/// only the vanilla installation's own config — never a world template.
struct Watchdog {
    /// The `server.properties` that was edited.
    path: PathBuf,
    /// Its full original text, written back on drop.
    original: String,
}

impl Watchdog {
    /// Sets `max-tick-time=-1`, remembering the whole original file.
    fn off(vanilla_dir: &Path) -> Result<Self, String> {
        let path = vanilla_dir.join("server.properties");
        let original = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let mut lines: Vec<String> = original.lines().map(str::to_owned).collect();
        let mut found = false;
        for line in &mut lines {
            if line.starts_with("max-tick-time=") {
                *line = "max-tick-time=-1".to_owned();
                found = true;
                break;
            }
        }
        if !found {
            lines.push("max-tick-time=-1".to_owned());
        }
        let mut body = lines.join("\n");
        body.push('\n');
        std::fs::write(&path, body)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        Ok(Self { path, original })
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Err(error) = std::fs::write(&self.path, &self.original) {
            eprintln!("cannot restore {}: {error}", self.path.display());
        }
    }
}

#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn build_world_template_v3() {
    let env = match parity_env_or_skip() {
        Some(env) => env,
        None => return,
    };
    if let Err(error) = std::fs::create_dir_all(TRACE_DIR) {
        eprintln!("SKIP: cannot create {TRACE_DIR}: {error}");
        return;
    }

    // Force-loading hundreds of chunks makes single ticks long; vanilla's
    // default 60 s watchdog force-shut the server down on the first attempt.
    // `max-tick-time=-1` is the standard server property for that; the guard
    // restores `server.properties` verbatim when it drops.
    let _watchdog = match Watchdog::off(&env.vanilla_dir) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("cannot disable the vanilla watchdog: {error}");
            return;
        }
    };

    let (x0, x1, z0, z1) = area();
    let mut report = String::new();
    report.push_str("# M4.0 (а), ЧАСТЬ III — world template v3\n\n");
    report.push_str(&format!(
        "- target: {side}×{side} = {want} chunks, centre chunk {CENTRE:?} \
         (the v2 ready-zone centre), x {x0}..{x1}, z {z0}..{z1}\n",
        side = 2 * HALF + 1,
        want = area_chunks(),
    ));
    report.push_str("\n- watchdog: `max-tick-time=-1` during the run (restored afterwards)\n");

    // --- phase probe: the per-command chunk limit, measured --------------
    let started = std::time::Instant::now();
    let mut vanilla = match Vanilla::start(&env).await {
        Ok(server) => server,
        Err(error) => {
            eprintln!("vanilla did not start for the probe: {error}");
            return;
        }
    };
    let log = vanilla.log_path().to_path_buf();
    let mut console = Console {
        vanilla: &mut vanilla,
        log,
        seq: 0,
    };
    let limit = match probe_limit(&mut console).await {
        Ok((limit, lines)) => {
            report.push_str("\n## probe: per-command `forceload add` limit\n\n");
            for line in &lines {
                report.push_str(&format!("- {line}\n"));
            }
            if limit == u64::MAX {
                report.push_str(&format!(
                    "- a single command accepted {PROBE_HIGH} chunks: no practical limit below \
                     the target\n"
                ));
            } else {
                report.push_str(&format!("- largest accepted strip: {limit} chunks\n"));
            }
            limit
        }
        Err(error) => {
            report.push_str(&format!("- probe failed: {error}\n"));
            eprintln!("probe failed: {error}");
            256
        }
    };
    drop(console);
    let _ = vanilla.stop_with_flush().await;
    eprintln!(
        "probe done in {:.1}s; limit={limit}",
        started.elapsed().as_secs_f64()
    );

    // --- phase build: generate the target on a fresh copy ----------------
    let build_started = std::time::Instant::now();
    let mut vanilla = match Vanilla::start(&env).await {
        Ok(server) => server,
        Err(error) => {
            eprintln!("vanilla did not start for the build: {error}");
            return;
        }
    };
    let log = vanilla.log_path().to_path_buf();
    let mut console = Console {
        vanilla: &mut vanilla,
        log: log.clone(),
        seq: 1_000,
    };
    let world = env.vanilla_dir.join("world");

    let side = (2 * HALF + 1) as u64;
    let strip_h = (limit / side).clamp(1, side) as i32;
    let mut commands = Vec::new();
    let mut cz = z0;
    while cz <= z1 {
        let cz1 = (cz + strip_h - 1).min(z1);
        let (bx0, bz0, bx1, bz1) = blocks(x0, x1, cz, cz1);
        commands.push(format!("forceload add {bx0} {bz0} {bx1} {bz1}"));
        cz = cz1 + 1;
    }
    report.push_str(&format!(
        "\n## build\n\n- forceload strip height {strip_h} chunks ({} commands, each ≤ {limit} chunks)\n",
        commands.len()
    ));
    // Pace the commands one by one: a single burst made one server tick exceed
    // the watchdog on the first attempt (before the watchdog was disabled).
    // One `sync` per command also proves each strip was accepted.
    let mut sent = 0usize;
    let mut failure = None;
    for (index, command) in commands.iter().enumerate() {
        match console.sync(std::slice::from_ref(command)).await {
            Ok(delta) => {
                sent += 1;
                eprintln!(
                    "forceload {}/{}: {}",
                    index + 1,
                    commands.len(),
                    notable(&delta)
                );
            }
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
        tokio::time::sleep(FORCELOAD_PACE).await;
    }
    match failure {
        Some(error) => {
            report.push_str(&format!(
                "- forceload stopped after {sent}/{} commands: {error}\n",
                commands.len()
            ));
            eprintln!(
                "forceload stopped after {sent}/{} commands: {error}",
                commands.len()
            );
        }
        None => report.push_str(&format!(
            "- all {} forceload commands accepted (one per sync)\n",
            commands.len()
        )),
    }

    // Generate: periodically save and rescan, stop when the area is full or
    // two consecutive scans show the same count.
    let mut last_full = 0usize;
    let mut stable = 0usize;
    let mut rounds = 0usize;
    for round in 1..=GEN_CHECKS {
        rounds = round;
        tokio::time::sleep(GEN_PERIOD).await;
        if let Err(error) = console.sync(&["save-all flush".to_owned()]).await {
            report.push_str(&format!("- save round {round} failed: {error}\n"));
        }
        match scan(&world) {
            Ok(current) => {
                eprintln!(
                    "round {round}: full={} in_area={}/{} elapsed={:.0}s",
                    current.full,
                    current.full_in_area,
                    area_chunks(),
                    build_started.elapsed().as_secs_f64()
                );
                let done = current.full_in_area as u64 >= area_chunks();
                stable = if current.full == last_full {
                    stable + 1
                } else {
                    0
                };
                last_full = current.full;
                if done || stable >= 2 {
                    break;
                }
            }
            Err(error) => {
                report.push_str(&format!("- scan round {round} failed: {error}\n"));
                eprintln!("scan round {round} failed: {error}");
            }
        }
    }
    let generation_secs = build_started.elapsed().as_secs_f64();

    // Remove all force-loads and prove the query is empty.
    let remove = console.sync(&["forceload remove all".to_owned()]).await;
    let query = console.sync(&["forceload query".to_owned()]).await;
    match (&remove, &query) {
        (Ok(removed), Ok(queried)) => {
            // Observed vanilla 26.3 reply: "No force loaded chunks were found
            // in minecraft:overworld" — the older "No chunks" wording is kept
            // as well so a wording change makes the check fail loudly.
            let empty = queried.contains("No force loaded chunks") || queried.contains("No chunks");
            report.push_str(&format!(
                "\n### force-load cleanup\n\n- remove all: {}\n- query empty: {empty} | {}\n",
                notable(removed),
                notable(queried)
            ));
            if !empty {
                eprintln!("WARNING: forceload query is not empty after remove all");
            }
        }
        (Err(error), _) | (_, Err(error)) => {
            report.push_str(&format!("\n### force-load cleanup\n\n- failed: {error}\n"));
        }
    }

    drop(console);
    if let Err(error) = vanilla.stop_with_flush().await {
        eprintln!("vanilla did not stop cleanly: {error}");
    }

    // --- collect: move the world to the v3 path --------------------------
    let v3 = PathBuf::from(V3_DIR);
    if v3.exists() {
        if let Err(error) = std::fs::remove_dir_all(&v3) {
            eprintln!("cannot clear {V3_DIR}: {error}");
            return;
        }
    }
    if let Err(error) = copy_dir_all(&world, &v3) {
        eprintln!("cannot copy the world to {V3_DIR}: {error}");
        return;
    }
    let size = dir_bytes(&v3).unwrap_or(0);

    // --- facts: scan v3, scan v2, compare --------------------------------
    let v3_scan = scan(&v3).unwrap_or_default();
    let v2_scan = scan(&env.world_template).unwrap_or_default();
    let compare = Command::new("python")
        .arg(chunk_status_script())
        .arg("compare")
        .arg(&env.world_template)
        .arg(&v3)
        .output();
    let compare_text = match compare {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).into_owned()
        }
        Ok(output) => format!(
            "compare failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => format!("compare could not run: {error}"),
    };

    report.push_str(&format!(
        "\n## facts\n\n\
         - generation wall time: {generation_secs:.0} s over {rounds} save/rescan rounds\n\
         - v3 size on disk: {}\n\
         - v3 chunks present: {} (bbox {:?})\n\
         - v3 `minecraft:full`: {} total, {} inside the target area (want {})\n\
         - v3 ready zone (bbox of `full`): {:?}\n\
         - v3 statuses: {:?}\n\
         - v2 chunks present: {} (`minecraft:full` {}, `initialize_light` {})\n",
        mb(size),
        v3_scan.total,
        v3_scan.bbox,
        v3_scan.full,
        v3_scan.full_in_area,
        area_chunks(),
        v3_scan.full_bbox,
        v3_scan.statuses,
        v2_scan.total,
        v2_scan.full,
        v2_scan.status("minecraft:initialize_light"),
    ));

    // Write the raw scan and compare output next to the traces.
    for (name, body) in [
        ("part3-v3-scan.txt", scan_text(&v3)),
        ("part3-v2-scan.txt", scan_text(&env.world_template)),
        ("part3-compare.txt", compare_text.clone()),
    ] {
        let _ = std::fs::write(Path::new(TRACE_DIR).join(name), body);
    }
    report.push_str("\n## compare v2 -> v3 (chunk_status.py compare)\n\n```\n");
    report.push_str(&compare_text);
    report.push_str("```\n");

    let summary = format!(
        "- v3: full {} (in area {}), present {}, size {}, generation {:.0}s\n\
         - compare: {}\n",
        v3_scan.full,
        v3_scan.full_in_area,
        v3_scan.total,
        mb(size),
        generation_secs,
        compare_text.lines().take(6).collect::<Vec<_>>().join(" | "),
    );
    let _ = std::fs::write(Path::new(TRACE_DIR).join("part3-report.md"), &report);
    let _ = std::fs::write(Path::new(TRACE_DIR).join("part3-summary.txt"), &summary);
    eprintln!("report: {TRACE_DIR}\\part3-report.md\n{summary}");
}

/// The raw `chunk_status.py scan` text of one world.
fn scan_text(world: &Path) -> String {
    match Command::new("python")
        .arg(chunk_status_script())
        .arg("scan")
        .arg(world)
        .output()
    {
        Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
        Err(error) => format!("scan could not run: {error}"),
    }
}
