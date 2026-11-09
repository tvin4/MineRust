//! World criterion of ЧАСТЬ I, п. 5: `chunk_status.py compare <template>
//! <copy>` decides what a run wrote. Only *new* chunks (`only_in_B`)
//! outside the owner's zone stop the sequence; every `status_diffs` row is
//! listed in the report and never stops it on its own. Chunk counts and
//! the heightmap/light diff counters are reported as facts.
//!
//! The script is a diagnostic helper outside the repository build
//! (`target/parity/chunk_status.py`); this module only runs it and reads
//! its printed Python literals — no region file is parsed here.

use std::path::Path;
use std::process::Command;

/// Allowed zone of new chunks: x −7..=19, z −15..=11 (owner's round-2
/// numbers, docs/ROUND2-CONTEXT.md).
pub const ZONE_X: (i32, i32) = (-7, 19);
pub const ZONE_Z: (i32, i32) = (-15, 11);

/// Chunk coordinate `(x, z)`.
pub type Coord = (i32, i32);

/// One `status_diffs` row: a chunk both sides have, with its two Statuses.
#[derive(Debug, Clone)]
pub struct StatusDiff {
    /// Chunk coordinates.
    pub at: Coord,
    /// Status in the template.
    pub from: String,
    /// Status in the copy.
    pub to: String,
}

/// What one `chunk_status.py compare` invocation proved. Fields keep their
/// defaults when `error` is set — nothing is invented instead of the
/// script's output.
#[derive(Debug, Default)]
pub struct WorldCriterion {
    /// Chunks only the template has (`only_in_A`): missing from the copy.
    pub missing_chunks: Vec<Coord>,
    /// Chunks only the copy has (`only_in_B`): new after the run.
    pub new_chunks: Vec<Coord>,
    /// Chunks present on both sides with a different Status.
    pub status_diffs: Vec<StatusDiff>,
    /// Chunks the script counted in the template.
    pub template_chunks: usize,
    /// Chunks the script counted in the copy.
    pub copy_chunks: usize,
    /// Heightmap diff rows printed by the script (information).
    pub heightmap_diffs: usize,
    /// Light presence diff rows printed by the script (information).
    pub light_diffs: usize,
    /// The script could not be run or its output did not parse: the
    /// criterion is unproven and the caller must stop.
    pub error: Option<String>,
}

impl WorldCriterion {
    /// Whether a chunk lies inside the owner's zone.
    pub const fn in_zone(at: Coord) -> bool {
        at.0 >= ZONE_X.0 && at.0 <= ZONE_X.1 && at.1 >= ZONE_Z.0 && at.1 <= ZONE_Z.1
    }

    /// New chunks inside the zone (allowed, listed without stopping).
    pub fn new_inside_zone(&self) -> Vec<Coord> {
        self.new_chunks
            .iter()
            .copied()
            .filter(|at| Self::in_zone(*at))
            .collect()
    }

    /// New chunks outside the zone — the only world stop of п. 5.
    pub fn new_outside_zone(&self) -> Vec<Coord> {
        self.new_chunks
            .iter()
            .copied()
            .filter(|at| !Self::in_zone(*at))
            .collect()
    }

    /// Status changes of chunks inside the zone (listed, no stop).
    pub fn status_inside_zone(&self) -> Vec<&StatusDiff> {
        self.status_diffs
            .iter()
            .filter(|d| Self::in_zone(d.at))
            .collect()
    }

    /// Status changes of chunks outside the zone (listed, no stop).
    pub fn status_outside_zone(&self) -> Vec<&StatusDiff> {
        self.status_diffs
            .iter()
            .filter(|d| !Self::in_zone(d.at))
            .collect()
    }
}

/// Runs `chunk_status.py compare <template> <world>` and parses its
/// stdout. Never panics: a missing `python`, a non-zero exit or
/// unparseable output becomes [`WorldCriterion::error`].
pub fn compare(script: &Path, template: &Path, world: &Path) -> WorldCriterion {
    let mut last_spawn = String::new();
    let mut output = None;
    for program in ["python", "py"] {
        let tried = Command::new(program)
            .arg(script)
            .arg("compare")
            .arg(template)
            .arg(world)
            .output();
        match tried {
            Ok(value) => {
                output = Some(value);
                break;
            }
            Err(error) => last_spawn = format!("{program}: {error}"),
        }
    }
    let Some(output) = output else {
        return WorldCriterion {
            error: Some(format!(
                "cannot run {}; not on PATH: {last_spawn}",
                script.display()
            )),
            ..WorldCriterion::default()
        };
    };
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if detail.is_empty() {
            format!("exit status {}", output.status)
        } else {
            detail
        };
        return WorldCriterion {
            error: Some(format!("chunk_status.py failed: {detail}")),
            ..WorldCriterion::default()
        };
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    match parse(&stdout) {
        Ok(mut case) => {
            if case.template_chunks == 0 {
                // The template scan is the baseline: if it read no chunk
                // (wrong tree, unreadable region dir), `only_in_B` is a
                // fiction and the criterion proves nothing.
                case.error = Some("the template scan found 0 chunks".to_owned());
            }
            case
        }
        Err(error) => WorldCriterion {
            error: Some(error),
            ..WorldCriterion::default()
        },
    }
}

/// Reads one `key=value` line of the script's stdout.
fn line_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(key))
}

/// The `... chunks=N` tail of an `A=`/`B=` line.
fn chunk_count(value: &str) -> Option<usize> {
    value.rsplit("chunks=").next()?.trim().parse::<usize>().ok()
}

/// Parses the script's stdout into the criterion. Any missing key is an
/// error: partial facts are not enough to decide the stop.
pub fn parse(text: &str) -> Result<WorldCriterion, String> {
    let missing = |key: &str| format!("compare output lacks `{key}=`");
    let new_raw = line_value(text, "only_in_B=").ok_or_else(|| missing("only_in_B"))?;
    let gone_raw = line_value(text, "only_in_A=").ok_or_else(|| missing("only_in_A"))?;
    let status_raw = line_value(text, "status_diffs=").ok_or_else(|| missing("status_diffs"))?;
    let template_chunks = line_value(text, "A=")
        .and_then(chunk_count)
        .ok_or_else(|| missing("A= chunks"))?;
    let copy_chunks = line_value(text, "B=")
        .and_then(chunk_count)
        .ok_or_else(|| missing("B= chunks"))?;
    let heightmap_diffs = line_value(text, "heightmap_diffs=")
        .map(|value| {
            value
                .split(':')
                .next()
                .unwrap_or_default()
                .trim()
                .parse::<usize>()
                .unwrap_or_default()
        })
        .ok_or_else(|| missing("heightmap_diffs"))?;
    let light_diffs = line_value(text, "light_presence_diffs=")
        .map(count_pairs)
        .ok_or_else(|| missing("light_presence_diffs"))?;

    Ok(WorldCriterion {
        missing_chunks: coord_pairs(gone_raw),
        new_chunks: coord_pairs(new_raw),
        status_diffs: status_rows(status_raw),
        template_chunks,
        copy_chunks,
        heightmap_diffs,
        light_diffs,
        error: None,
    })
}

/// All `(x, z)` pairs of a Python list literal such as `[(1, 2), (3, 4)]`.
fn coord_pairs(value: &str) -> Vec<Coord> {
    let mut out = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'(' {
            if let Some((at, next)) = pair_at(value, index) {
                out.push(at);
                index = next;
                continue;
            }
        }
        index += 1;
    }
    out
}

/// Parses `x, z` right after the `(` at `start`; returns the pair and the
/// index just past its `)`.
fn pair_at(value: &str, start: usize) -> Option<(Coord, usize)> {
    if value.as_bytes().get(start) != Some(&b'(') {
        return None;
    }
    let rest = &value[start + 1..];
    let end = rest.find(')')?;
    let (first, second) = rest[..end].split_once(',')?;
    let at = (first.trim().parse().ok()?, second.trim().parse().ok()?);
    Some((at, start + 1 + end + 1))
}

/// Number of `((x, z), ...)` rows in one list literal.
fn count_pairs(value: &str) -> usize {
    value.matches("((").count()
}

/// One `status_diffs` row: `((x, z), 'from', 'to')`.
fn status_rows(value: &str) -> Vec<StatusDiff> {
    let mut out = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b'(' && bytes[index + 1] == b'(' {
            if let Some((at, next)) = pair_at(value, index + 1) {
                if let Some((from, to)) = quoted_pair(&value[next..]) {
                    out.push(StatusDiff { at, from, to });
                    index = next;
                    continue;
                }
            }
        }
        index += 1;
    }
    out
}

/// Reads `, 'from', 'to'` after a parsed coordinate pair.
fn quoted_pair(rest: &str) -> Option<(String, String)> {
    let from = quoted(rest)?;
    let to = quoted(&rest[from.1..])?;
    Some((from.0, to.0))
}

/// One `'...'` literal: the text plus the index just past its closing
/// quote, counted from the start of `rest`.
fn quoted(rest: &str) -> Option<(String, usize)> {
    let start = rest.find('\'')? + 1;
    let end = start + rest[start..].find('\'')?;
    Some((rest[start..end].to_owned(), end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Output of `chunk_status.py compare` in the shape the script prints.
    const SAMPLE: &str = "\
A=C:\\MineRust-data\\world-template chunks=4
B=C:\\MineRust-data\\vanilla\\world chunks=2
only_in_A=[(0, 1)]
only_in_B=[(3, -1), (-8, 12)]
status_diffs=[((3, -1), 'from', 'to'), ((50, 50), 'from', 'to')]
heightmap_diffs=2: [(1, 1), (2, 2)]
light_presence_diffs=[((3, -1), (15, 15), (0, 0))]
";

    #[test]
    fn the_zone_is_the_owners_rectangle() {
        assert!(WorldCriterion::in_zone((-7, -15)));
        assert!(WorldCriterion::in_zone((19, 11)));
        assert!(WorldCriterion::in_zone((3, -1)));
        assert!(!WorldCriterion::in_zone((-8, -15)));
        assert!(!WorldCriterion::in_zone((20, 11)));
        assert!(!WorldCriterion::in_zone((3, 12)));
    }

    #[test]
    fn the_compare_output_splits_into_zone_and_facts() {
        let case = parse(SAMPLE).expect("sample parses");
        assert_eq!(case.template_chunks, 4);
        assert_eq!(case.copy_chunks, 2);
        assert_eq!(case.missing_chunks, vec![(0, 1)]);
        assert_eq!(case.new_chunks, vec![(3, -1), (-8, 12)]);
        assert_eq!(case.new_inside_zone(), vec![(3, -1)]);
        assert_eq!(case.new_outside_zone(), vec![(-8, 12)]);
        assert_eq!(case.heightmap_diffs, 2);
        assert_eq!(case.light_diffs, 1);

        let inside = case.status_inside_zone();
        assert_eq!(inside.len(), 1, "the (3, -1) row is inside the zone");
        assert_eq!(
            (inside[0].from.as_str(), inside[0].to.as_str()),
            ("from", "to")
        );
        let outside = case.status_outside_zone();
        assert_eq!(outside.len(), 1);
        assert_eq!(outside[0].at, (50, 50));
    }

    #[test]
    fn a_missing_key_is_refused_rather_than_guessed() {
        let truncated = "only_in_B=[(3, -1)]\n";
        let error = parse(truncated).expect_err("the rest of the keys is absent");
        assert!(error.contains("only_in_A="), "got: {error}");
    }

    #[test]
    fn empty_lists_parse_to_no_rows() {
        let case = parse(
            "A=x chunks=1\nB=x chunks=1\nonly_in_A=[]\nonly_in_B=[]\n\
             status_diffs=[]\nheightmap_diffs=0: []\nlight_presence_diffs=[]\n",
        )
        .expect("empty lists parse");
        assert!(case.new_chunks.is_empty());
        assert!(case.status_diffs.is_empty());
        assert_eq!(case.heightmap_diffs, 0);
    }
}
