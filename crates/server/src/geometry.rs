//! Development-only block geometry from a list of `/setblock` and `/fill`
//! commands (Шаг 15 п.2).
//!
//! Disabled unless `dev_geometry` in `server.toml` names a file. When it is
//! set the server warns once, reads the file at startup and writes every
//! command into the in-memory [`ChunkStore`](net::ChunkStore) before the port
//! is bound. The world directory on disk is never touched: the store is the
//! wire snapshot the connections send, rebuilt in memory only.
//!
//! The accepted syntax is the one the S10 vanilla stand fed to the real
//! server (`E:\MineRust-data\autotests\s10-vanilla-r3\s10-samples.json`):
//!
//! ```text
//! setblock x y z name[key=value,...]
//! fill x1 y1 z1 x2 y2 z2 name[key=value,...]
//! ```
//!
//! An optional leading `/`, blank lines and `#` comments are allowed. A block
//! name without a namespace is read as `minecraft:`. An unknown name or a
//! property combination no state accepts is an error, never a guess.
//!
//! SOURCE: the stand's command list; the parser is written from that format,
//! not copied from the game.

use std::path::Path;

use data::{BlockStates, Properties};
use net::ChunkStore;

/// Errors while parsing or applying a dev geometry file.
#[derive(Debug, thiserror::Error)]
pub enum GeometryError {
    /// The command file could not be read.
    #[error("failed to read dev geometry file {path}: {source}")]
    Io {
        /// Path that could not be read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A line is not a valid `setblock`/`fill` command.
    #[error("dev geometry line {line}: {message}")]
    Syntax {
        /// 1-based line number.
        line: usize,
        /// What is wrong with the line.
        message: String,
    },
    /// The block name or its state is not in the blocks report.
    #[error("dev geometry line {line}: unknown block `{block}`")]
    UnknownBlock {
        /// 1-based line number.
        line: usize,
        /// The block spec as written.
        block: String,
    },
    /// The block write failed (missing chunk or section).
    #[error("dev geometry line {line}: {source}")]
    Write {
        /// 1-based line number.
        line: usize,
        /// The store error.
        #[source]
        source: net::StoreError,
    },
}

/// A block as written in a command: a name plus the properties given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockSpec {
    /// Namespaced block name (`minecraft:stone`; a bare name gets the
    /// `minecraft:` namespace).
    pub name: String,
    /// The properties written in `[...]`, possibly empty.
    pub properties: Properties,
}

impl BlockSpec {
    /// The spec as it would be written back (`name` or `name[k=v,...]`).
    pub fn render(&self) -> String {
        if self.properties.is_empty() {
            return self.name.clone();
        }
        let inner: Vec<String> = self
            .properties
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        format!("{}[{}]", self.name, inner.join(","))
    }
}

/// One parsed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `/setblock x y z block`
    Set {
        /// Block x.
        x: i32,
        /// Block y.
        y: i32,
        /// Block z.
        z: i32,
        /// The block to place.
        block: BlockSpec,
    },
    /// `/fill x1 y1 z1 x2 y2 z2 block` (the two corners in any order).
    Fill {
        /// First corner.
        from: (i32, i32, i32),
        /// Opposite corner.
        to: (i32, i32, i32),
        /// The block to place.
        block: BlockSpec,
    },
}

impl Command {
    /// The block of the command.
    pub fn block(&self) -> &BlockSpec {
        match self {
            Command::Set { block, .. } | Command::Fill { block, .. } => block,
        }
    }
}

/// Parses one command line.
///
/// A leading `/`, blank space around the tokens and a block name without a
/// namespace (`stone` → `minecraft:stone`) are accepted. `Err` carries a
/// human-readable message without a line number; [`load`] adds it.
pub fn parse_command(line: &str) -> Result<Command, String> {
    let line = line.trim();
    let line = line.strip_prefix('/').unwrap_or(line);
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let (subcommand, rest) = tokens
        .split_first()
        .ok_or_else(|| "empty command".to_owned())?;
    match *subcommand {
        "setblock" => {
            require_tokens(rest, 4, "setblock x y z block")?;
            Ok(Command::Set {
                x: parse_coordinate(rest[0])?,
                y: parse_coordinate(rest[1])?,
                z: parse_coordinate(rest[2])?,
                block: parse_block(rest[3])?,
            })
        }
        "fill" => {
            require_tokens(rest, 7, "fill x1 y1 z1 x2 y2 z2 block")?;
            Ok(Command::Fill {
                from: (
                    parse_coordinate(rest[0])?,
                    parse_coordinate(rest[1])?,
                    parse_coordinate(rest[2])?,
                ),
                to: (
                    parse_coordinate(rest[3])?,
                    parse_coordinate(rest[4])?,
                    parse_coordinate(rest[5])?,
                ),
                block: parse_block(rest[6])?,
            })
        }
        other => Err(format!("expected `setblock` or `fill`, got `{other}`")),
    }
}

/// Checks that a command has exactly `count` arguments.
fn require_tokens(tokens: &[&str], count: usize, shape: &str) -> Result<(), String> {
    if tokens.len() == count {
        Ok(())
    } else {
        Err(format!(
            "expected {shape} ({count} arguments), got {} token(s)",
            tokens.len()
        ))
    }
}

/// Parses one 32-bit coordinate.
fn parse_coordinate(token: &str) -> Result<i32, String> {
    token
        .parse::<i32>()
        .map_err(|_| format!("coordinate is not a 32-bit integer: `{token}`"))
}

/// Parses `name[key=value,...]` into a [`BlockSpec`].
fn parse_block(token: &str) -> Result<BlockSpec, String> {
    let (name, properties) = match token.split_once('[') {
        None => (token, Properties::new()),
        Some((name, tail)) => {
            let inner = tail
                .strip_suffix(']')
                .ok_or_else(|| format!("block state `{token}` has no closing `]`"))?;
            (name, parse_properties(inner)?)
        }
    };
    if name.is_empty() {
        return Err("block name is empty".to_owned());
    }
    let name = if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    };
    Ok(BlockSpec { name, properties })
}

/// Parses the comma-separated `key=value` list inside `[...]`.
fn parse_properties(inner: &str) -> Result<Properties, String> {
    let mut properties = Properties::new();
    if inner.trim().is_empty() {
        return Ok(properties);
    }
    for pair in inner.split(',') {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("block property `{pair}` is not `key=value`"))?;
        let (key, value) = (key.trim(), value.trim());
        if key.is_empty() || value.is_empty() {
            return Err(format!("block property `{pair}` is not `key=value`"));
        }
        properties.insert(key.to_owned(), value.to_owned());
    }
    Ok(properties)
}

/// The state id of `spec` under the blocks report, or `None` when the report
/// does not know the name or accepts no state with those properties.
pub fn resolve_state(blocks: &BlockStates, spec: &BlockSpec) -> Option<i32> {
    blocks.state_id_with_overrides(&spec.name, &spec.properties)
}

/// Parses every command of `path`, skipping blank lines and `#` comments.
pub fn load(path: impl AsRef<Path>) -> Result<Vec<(usize, Command)>, GeometryError> {
    let path_ref = path.as_ref();
    let display = path_ref.display().to_string();
    let text = std::fs::read_to_string(path_ref).map_err(|source| GeometryError::Io {
        path: display,
        source,
    })?;
    // A file written by Windows PowerShell starts with a UTF-8 BOM; it is not
    // part of the first command.
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut commands = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let command =
            parse_command(trimmed).map_err(|message| GeometryError::Syntax { line, message })?;
        commands.push((line, command));
    }
    Ok(commands)
}

/// Applies one resolved command to the store; returns how many cells it
/// touched.
pub fn apply(store: &ChunkStore, command: &Command, state: i32) -> Result<usize, net::StoreError> {
    match command {
        Command::Set { x, y, z, .. } => {
            store.set_block(*x, *y, *z, state)?;
            Ok(1)
        }
        Command::Fill { from, to, .. } => {
            let mut touched = 0;
            for x in axis(from.0, to.0) {
                for y in axis(from.1, to.1) {
                    for z in axis(from.2, to.2) {
                        store.set_block(x, y, z, state)?;
                        touched += 1;
                    }
                }
            }
            Ok(touched)
        }
    }
}

/// The inclusive integer span between two corners of an axis.
fn axis(first: i32, second: i32) -> std::ops::RangeInclusive<i32> {
    first.min(second)..=first.max(second)
}

/// Loads, resolves and applies a whole geometry file.
///
/// Returns how many cells were written. The first bad line aborts with its
/// line number, before any of it reaches a client (the caller runs this
/// before the port is bound).
pub fn apply_file(
    store: &ChunkStore,
    blocks: &BlockStates,
    path: impl AsRef<Path>,
) -> Result<usize, GeometryError> {
    let commands = load(path)?;
    let mut touched = 0;
    for (line, command) in &commands {
        let spec = command.block();
        let state = resolve_state(blocks, spec).ok_or_else(|| GeometryError::UnknownBlock {
            line: *line,
            block: spec.render(),
        })?;
        touched += apply(store, command, state).map_err(|source| GeometryError::Write {
            line: *line,
            source,
        })?;
    }
    Ok(touched)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small report: stone, a two-state slab and a three-state column whose
    /// default carries properties.
    fn blocks() -> BlockStates {
        let text = r#"{
          "minecraft:stone": {"states": [{"id": 1, "default": true}]},
          "minecraft:oak_slab": {"states": [
            {"id": 10, "default": true, "properties": {"type": "bottom"}},
            {"id": 11, "properties": {"type": "top"}}
          ]},
          "minecraft:bubble_column": {"states": [
            {"id": 20, "default": true,
             "properties": {"drag": "false", "waterlogged": "true"}},
            {"id": 21, "properties": {"drag": "true", "waterlogged": "true"}}
          ]}
        }"#;
        BlockStates::parse(text, "test".to_owned()).expect("valid report")
    }

    #[test]
    fn parses_a_setblock_with_and_without_properties() {
        assert_eq!(
            parse_command("/setblock 100 201 -24 minecraft:bubble_column[drag=true]")
                .expect("parsed"),
            Command::Set {
                x: 100,
                y: 201,
                z: -24,
                block: BlockSpec {
                    name: "minecraft:bubble_column".to_owned(),
                    properties: Properties::from([("drag".to_owned(), "true".to_owned())]),
                },
            }
        );
        assert_eq!(
            parse_command("setblock 0 -64 0 stone").expect("parsed"),
            Command::Set {
                x: 0,
                y: -64,
                z: 0,
                block: BlockSpec {
                    name: "minecraft:stone".to_owned(),
                    properties: Properties::new(),
                },
            },
            "a bare name reads as minecraft:"
        );
    }

    #[test]
    fn parses_a_fill_and_two_properties() {
        assert_eq!(
            parse_command(
                "fill 87 200 -35 89 200 -33 minecraft:stone_slab[type=bottom,waterlogged=true]"
            )
            .expect("parsed"),
            Command::Fill {
                from: (87, 200, -35),
                to: (89, 200, -33),
                block: BlockSpec {
                    name: "minecraft:stone_slab".to_owned(),
                    properties: Properties::from([
                        ("type".to_owned(), "bottom".to_owned()),
                        ("waterlogged".to_owned(), "true".to_owned()),
                    ]),
                },
            }
        );
    }

    #[test]
    fn rejects_malformed_commands() {
        for line in [
            "",
            "teleport 1 2 3",
            "setblock 1 2 minecraft:stone",
            "setblock 1 2 3",
            "setblock 1.5 2 3 minecraft:stone",
            "fill 1 2 3 4 5 minecraft:stone",
            "setblock 1 2 3 minecraft:stone[type=top",
            "setblock 1 2 3 minecraft:stone[type]",
            "setblock 1 2 3 minecraft:stone[]extra",
        ] {
            assert!(parse_command(line).is_err(), "accepted `{line}`");
        }
    }

    #[test]
    fn resolves_states_with_default_fill_and_reports_unknowns() {
        let blocks = blocks();
        let stone = BlockSpec {
            name: "minecraft:stone".to_owned(),
            properties: Properties::new(),
        };
        assert_eq!(resolve_state(&blocks, &stone), Some(1));

        // `drag=true` alone: `waterlogged` comes from the default state.
        let drag = BlockSpec {
            name: "minecraft:bubble_column".to_owned(),
            properties: Properties::from([("drag".to_owned(), "true".to_owned())]),
        };
        assert_eq!(resolve_state(&blocks, &drag), Some(21));

        let unknown = BlockSpec {
            name: "minecraft:nope".to_owned(),
            properties: Properties::new(),
        };
        assert_eq!(resolve_state(&blocks, &unknown), None);
        let bad = BlockSpec {
            name: "minecraft:oak_slab".to_owned(),
            properties: Properties::from([("type".to_owned(), "side".to_owned())]),
        };
        assert_eq!(resolve_state(&blocks, &bad), None);
    }

    /// A store holding one all-air chunk of the 24 section slots Y −4..19,
    /// classified against the fixture report.
    fn store_with_chunk(chunk_x: i32, chunk_z: i32) -> net::ChunkStore {
        use protocol::{
            ChunkSection, HeightmapRecord, LevelChunkWithLight, LightData, PaletteContainer,
        };
        let chunk = LevelChunkWithLight {
            chunk_x,
            chunk_z,
            heightmaps: vec![HeightmapRecord {
                id: 4,
                words: vec![0],
            }],
            sections: (0..24)
                .map(|_| ChunkSection {
                    non_air: 0,
                    fluid: 0,
                    blocks: PaletteContainer::Single(0),
                    biomes: PaletteContainer::Single(1),
                })
                .collect(),
            block_entities: Vec::new(),
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky_arrays: Vec::new(),
                block_arrays: Vec::new(),
            },
        };
        net::ChunkStore::new(
            net::ReadyChunks::new(vec![chunk]),
            std::sync::Arc::new(blocks()),
        )
    }

    #[test]
    fn applies_parsed_commands_into_a_chunk_store() {
        let store = store_with_chunk(0, 0);
        let blocks = blocks();
        let commands = [
            parse_command("setblock 8 200 8 minecraft:bubble_column[drag=true]").expect("parsed"),
            parse_command("fill 0 201 0 2 201 2 minecraft:stone").expect("parsed"),
        ];
        let mut touched = 0;
        for command in &commands {
            let state = resolve_state(&blocks, command.block()).expect("known block");
            touched += apply(&store, command, state).expect("writable");
        }
        assert_eq!(touched, 10, "1 setblock cell + 3*3 fill cells");
        assert_eq!(store.block(8, 200, 8), Some(21), "drag=true column");
        assert_eq!(store.block(0, 201, 0), Some(1), "fill corner");
        assert_eq!(store.block(2, 201, 2), Some(1), "opposite corner");
        assert_eq!(
            store.block(3, 201, 0),
            Some(0),
            "outside the fill stays air"
        );
    }

    #[test]
    fn apply_file_accepts_a_utf8_bom() {
        let store = store_with_chunk(0, 0);
        let path = std::env::temp_dir().join("minerust-dev-geometry-bom.txt");
        std::fs::write(&path, "\u{feff}setblock 7 200 7 stone\n").expect("write geometry file");
        let touched = apply_file(&store, store.blocks(), &path).expect("BOM is not a command");
        assert_eq!(touched, 1);
        assert_eq!(store.block(7, 200, 7), Some(1));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn apply_file_skips_comments_and_reports_the_bad_line() {
        let store = store_with_chunk(0, 0);
        let path = std::env::temp_dir().join("minerust-dev-geometry-test.txt");
        std::fs::write(
            &path,
            "# a comment\n\nsetblock 5 200 5 stone\nsetblock 6 200 6 minecraft:nope\n",
        )
        .expect("write geometry file");
        let error = apply_file(&store, store.blocks(), &path).expect_err("unknown block");
        match error {
            GeometryError::UnknownBlock { line, block } => {
                assert_eq!(line, 4, "the bad line is named");
                assert_eq!(block, "minecraft:nope");
            }
            other => panic!("unexpected error: {other}"),
        }
        // The line before the bad one was already applied.
        assert_eq!(store.block(5, 200, 5), Some(1), "the good line went in");
        let _ = std::fs::remove_file(&path);
    }
}
