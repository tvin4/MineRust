//! Clientbound command tree (Declare Commands, play id `0x10`).
//!
//! Step 25b sends the tree on join so the vanilla client can parse, tab
//! complete and validate `/help`, `/gamemode`, `/tp`, `/time` and `/say`.
//! Only the nodes this server actually implements are declared, trimmed by
//! the player's operator level: the vanilla non-operator tree of S21 has 26
//! nodes and its operator tree 34 KB, neither is reproduced here (the bytes
//! are data of the user's own game, see `docs/MECHANICS.md` 25.0c).
//!
//! SOURCE: 2026-11-18_s21-nonop-vanilla-step25-r2, conn-1 seq 52: the
//! node/flags/children/name layout is decoded from that 265-byte tree; the
//! 265 bytes parse as 26 nodes followed by the root index `00`, and the
//! `brigadier:` numeric parsers carry a flags byte (none here). The
//! per-parser property bytes follow the public protocol documentation.
//!
//! The trailing root index is what the client reads last: without it the
//! decoder hits EOF and disconnects with "Failed to decode packet
//! 'clientbound/minecraft:commands'" (our first join after 25b, 18.11.2026).

use crate::codec::{CodecError, PacketWriter};

/// Maximum characters of a node or argument name.
///
/// SOURCE: the protocol string limit for a command node name; every name
/// here is ASCII and far below it.
const MAX_NODE_NAME_CHARS: usize = 32767;

/// Node type bits of the flags byte.
const TYPE_ROOT: u8 = 0x00;
const TYPE_LITERAL: u8 = 0x01;
const TYPE_ARGUMENT: u8 = 0x02;

/// The node can finish the command.
const FLAG_EXECUTABLE: u8 = 0x04;

/// The node redirects to another node instead of carrying its own children.
const FLAG_REDIRECT: u8 = 0x08;

/// Parser id of `brigadier:string` in `minecraft:command_argument_type`.
// SOURCE: reports/registries.json of the user's game copy.
const PARSER_STRING: i32 = 5;

/// Parser id of `minecraft:entity`.
const PARSER_ENTITY: i32 = 6;

/// Parser id of `minecraft:vec3`.
const PARSER_VEC3: i32 = 10;

/// Parser id of `minecraft:message`.
const PARSER_MESSAGE: i32 = 20;

/// Parser id of `minecraft:gamemode`.
const PARSER_GAMEMODE: i32 = 42;

/// Parser id of `minecraft:time`.
const PARSER_TIME: i32 = 43;

/// Quotable string kinds of `brigadier:string`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringKind {
    /// One word, no spaces.
    SingleWord,
    /// A quoted phrase.
    QuotablePhrase,
    /// Everything up to the end of the line.
    GreedyPhrase,
}

impl StringKind {
    /// The property VarInt the wire uses.
    fn code(self) -> i32 {
        match self {
            Self::SingleWord => 0,
            Self::QuotablePhrase => 1,
            Self::GreedyPhrase => 2,
        }
    }
}

/// The parser of an argument node and its parser-specific properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgumentParser {
    /// `brigadier:string`.
    String(StringKind),
    /// `minecraft:entity`; `flags` bit `0x01` single, `0x02` players only.
    Entity {
        /// Entity selector flags byte.
        flags: u8,
    },
    /// `minecraft:vec3` (three doubles).
    Vec3,
    /// `minecraft:message` (a chat message, greedy).
    Message,
    /// `minecraft:gamemode` (the four mode names).
    GameMode,
    /// `minecraft:time` with its minimum accepted value.
    Time {
        /// Smallest accepted value.
        min: i32,
    },
}

impl ArgumentParser {
    /// The numeric parser id of `minecraft:command_argument_type`.
    fn id(&self) -> i32 {
        match self {
            Self::String(_) => PARSER_STRING,
            Self::Entity { .. } => PARSER_ENTITY,
            Self::Vec3 => PARSER_VEC3,
            Self::Message => PARSER_MESSAGE,
            Self::GameMode => PARSER_GAMEMODE,
            Self::Time { .. } => PARSER_TIME,
        }
    }

    /// Writes the parser-specific property bytes.
    fn write_properties(&self, writer: &mut PacketWriter) {
        match self {
            Self::String(kind) => writer.write_varint(kind.code()),
            Self::Entity { flags } => writer.write_u8(*flags),
            Self::Time { min } => writer.write_i32(*min),
            Self::Vec3 | Self::Message | Self::GameMode => {}
        }
    }
}

/// What a node is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandNodeKind {
    /// The root of the tree; must be the first node.
    Root,
    /// A literal (a word such as `gamemode`).
    Literal {
        /// The literal text.
        name: String,
    },
    /// An argument of a parser.
    Argument {
        /// Argument name shown by the client.
        name: String,
        /// Parser and its properties.
        parser: ArgumentParser,
    },
}

impl CommandNodeKind {
    /// The type bits of the flags byte.
    fn type_bits(&self) -> u8 {
        match self {
            Self::Root => TYPE_ROOT,
            Self::Literal { .. } => TYPE_LITERAL,
            Self::Argument { .. } => TYPE_ARGUMENT,
        }
    }
}

/// One node of a [`CommandTree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandNode {
    /// Root, literal or argument.
    pub kind: CommandNodeKind,
    /// Whether the command can end at this node.
    pub executable: bool,
    /// Indices of the child nodes.
    pub children: Vec<u32>,
    /// Redirect target, mutually exclusive with `children`.
    pub redirect: Option<u32>,
}

impl CommandNode {
    /// A literal node without children or executable flag.
    pub fn literal(name: &str) -> Self {
        Self {
            kind: CommandNodeKind::Literal {
                name: name.to_owned(),
            },
            executable: false,
            children: Vec::new(),
            redirect: None,
        }
    }

    /// An argument node.
    pub fn argument(name: &str, parser: ArgumentParser) -> Self {
        Self {
            kind: CommandNodeKind::Argument {
                name: name.to_owned(),
                parser,
            },
            executable: false,
            children: Vec::new(),
            redirect: None,
        }
    }

    /// Marks the node executable.
    pub fn executable(mut self) -> Self {
        self.executable = true;
        self
    }

    /// Adds one child index.
    pub fn child(mut self, index: u32) -> Self {
        self.children.push(index);
        self
    }
}

/// A complete command tree, root first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTree {
    /// The nodes in wire order; node zero is the root.
    pub nodes: Vec<CommandNode>,
}

impl CommandTree {
    /// Encodes the payload (packet id excluded).
    ///
    /// Fails on a tree the client could not read: an empty or non-root first
    /// node, a child or redirect index outside the list, or a node that both
    /// redirects and has children.
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let count =
            i32::try_from(self.nodes.len()).map_err(|_| CodecError::NegativeLength(i32::MAX))?;
        if !matches!(
            self.nodes.first().map(|node| &node.kind),
            Some(CommandNodeKind::Root)
        ) {
            return Err(CodecError::InvalidCommandTree(
                "the first node must be the root".to_owned(),
            ));
        }
        let mut writer = PacketWriter::new();
        writer.write_varint(count);
        for node in &self.nodes {
            self.write_node(&mut writer, node)?;
        }
        // The root is always node zero (checked above); the client reads this
        // index after the node list.
        writer.write_varint(0);
        Ok(writer.into_bytes())
    }

    /// Writes one node, validating its indices first.
    fn write_node(&self, writer: &mut PacketWriter, node: &CommandNode) -> Result<(), CodecError> {
        if node.redirect.is_some() && !node.children.is_empty() {
            return Err(CodecError::InvalidCommandTree(
                "a node cannot both redirect and have children".to_owned(),
            ));
        }
        for child in &node.children {
            self.check_index(*child)?;
        }
        let redirect = match node.redirect {
            Some(index) => {
                self.check_index(index)?;
                Some(index)
            }
            None => None,
        };
        let mut flags = node.kind.type_bits();
        if node.executable {
            flags |= FLAG_EXECUTABLE;
        }
        if redirect.is_some() {
            flags |= FLAG_REDIRECT;
        }
        writer.write_u8(flags);
        writer.write_varint(
            i32::try_from(node.children.len()).map_err(|_| CodecError::NegativeLength(i32::MAX))?,
        );
        for child in &node.children {
            writer.write_varint(*child as i32);
        }
        if let Some(index) = redirect {
            writer.write_varint(index as i32);
        }
        match &node.kind {
            CommandNodeKind::Root => {}
            CommandNodeKind::Literal { name } => {
                writer.write_string(name, MAX_NODE_NAME_CHARS)?;
            }
            CommandNodeKind::Argument { name, parser } => {
                writer.write_string(name, MAX_NODE_NAME_CHARS)?;
                writer.write_varint(parser.id());
                parser.write_properties(writer);
            }
        }
        Ok(())
    }

    /// Checks one child or redirect index.
    fn check_index(&self, index: u32) -> Result<(), CodecError> {
        if index as usize >= self.nodes.len() {
            return Err(CodecError::InvalidCommandTree(format!(
                "node index {index} is outside the tree of {} nodes",
                self.nodes.len()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A root with one executable literal.
    fn help_tree() -> CommandTree {
        CommandTree {
            nodes: vec![
                CommandNode {
                    kind: CommandNodeKind::Root,
                    executable: false,
                    children: vec![1],
                    redirect: None,
                },
                CommandNode::literal("help").executable(),
            ],
        }
    }

    #[test]
    fn a_two_node_tree_has_the_documented_layout() {
        // count 2, root flags 0 children 1 -> node 1, literal|executable 0x05,
        // no children, name length 4 and "help", then the root index 0.
        assert_eq!(
            help_tree().encode_payload().expect("encodable"),
            b"\x02\x00\x01\x01\x05\x00\x04help\x00"
        );
    }

    #[test]
    fn the_vanilla_tail_is_the_root_index_after_the_last_node() {
        // SOURCE: 2026-11-18_s21-nonop-vanilla-step25-r2, conn-1 seq 52: the
        // 265-byte payload ends `... 05 value 03 00 00`, i.e. the last
        // argument node followed by the root index. The client reads it after
        // the node list; without it the decoder hits EOF.
        let bytes = help_tree().encode_payload().expect("encodable");
        assert_eq!(
            bytes.last(),
            Some(&0x00),
            "the root index closes the payload"
        );
        assert_eq!(bytes.len(), 12, "two nodes plus the root index");
    }

    #[test]
    fn an_argument_carries_its_parser_and_properties() {
        let tree = CommandTree {
            nodes: vec![
                CommandNode {
                    kind: CommandNodeKind::Root,
                    executable: false,
                    children: vec![1],
                    redirect: None,
                },
                CommandNode::argument("mode", ArgumentParser::GameMode).executable(),
                CommandNode::argument("text", ArgumentParser::String(StringKind::GreedyPhrase)),
            ],
        };
        let bytes = tree.encode_payload().expect("encodable");
        // count 3, root: 00 01 01; mode: 06 00 04 "mode" 2a; text: 02 00 04
        // "text" 05 02.
        assert_eq!(
            bytes,
            b"\x03\x00\x01\x01\x06\x00\x04mode\x2a\x02\x00\x04text\x05\x02\x00"
        );
    }

    #[test]
    fn a_redirect_is_flagged_and_written_before_the_name() {
        let tree = CommandTree {
            nodes: vec![
                CommandNode {
                    kind: CommandNodeKind::Root,
                    executable: false,
                    children: vec![1, 2],
                    redirect: None,
                },
                CommandNode {
                    kind: CommandNodeKind::Literal {
                        name: "tell".to_owned(),
                    },
                    executable: false,
                    children: Vec::new(),
                    redirect: Some(2),
                },
                CommandNode::literal("msg").executable(),
            ],
        };
        let bytes = tree.encode_payload().expect("encodable");
        assert_eq!(
            bytes,
            b"\x03\x00\x02\x01\x02\x09\x00\x02\x04tell\x05\x00\x03msg\x00"
        );
    }

    #[test]
    fn a_tree_without_a_root_is_rejected() {
        let tree = CommandTree {
            nodes: vec![CommandNode::literal("help").executable()],
        };
        assert!(matches!(
            tree.encode_payload(),
            Err(CodecError::InvalidCommandTree(_))
        ));
    }

    #[test]
    fn an_index_outside_the_tree_is_rejected() {
        let mut tree = help_tree();
        tree.nodes[0].children.push(7);
        assert!(matches!(
            tree.encode_payload(),
            Err(CodecError::InvalidCommandTree(_))
        ));
    }
}
