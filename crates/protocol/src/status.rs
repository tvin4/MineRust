//! Server list status document.

/// Data shown for this server in the multiplayer server list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusInfo {
    /// Version name displayed next to the player count.
    pub version_name: String,
    /// Protocol version displayed to the client; the client treats the server
    /// as compatible only when this matches its own protocol version.
    pub protocol_version: i32,
    /// Maximum player count.
    pub max_players: i32,
    /// Currently online players.
    pub online_players: i32,
    /// Message of the day.
    pub motd: String,
}

impl StatusInfo {
    /// Renders the JSON document carried by the status response packet.
    ///
    /// The field layout follows the public protocol documentation: `version`
    /// (with mandatory `name`), `players` and `description` as a text
    /// component.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "version": { "name": self.version_name, "protocol": self.protocol_version },
            "players": { "max": self.max_players, "online": self.online_players },
            "description": { "text": self.motd },
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_exposes_version_players_and_motd() {
        let info = StatusInfo {
            version_name: "test-version".to_owned(),
            protocol_version: 4242,
            max_players: 20,
            online_players: 0,
            motd: "Hello, world!".to_owned(),
        };

        let value: serde_json::Value = serde_json::from_str(&info.to_json()).expect("valid json");
        assert_eq!(value["version"]["name"].as_str(), Some("test-version"));
        assert_eq!(value["version"]["protocol"].as_i64(), Some(4242));
        assert_eq!(value["players"]["max"].as_i64(), Some(20));
        assert_eq!(value["players"]["online"].as_i64(), Some(0));
        assert_eq!(value["description"]["text"].as_str(), Some("Hello, world!"));
    }

    #[test]
    fn json_escapes_special_characters() {
        let info = StatusInfo {
            version_name: "v".to_owned(),
            protocol_version: 1,
            max_players: 1,
            online_players: 0,
            motd: "quote \" and \\ and \n".to_owned(),
        };
        let value: serde_json::Value = serde_json::from_str(&info.to_json()).expect("valid json");
        assert_eq!(
            value["description"]["text"].as_str(),
            Some("quote \" and \\ and \n")
        );
    }
}
