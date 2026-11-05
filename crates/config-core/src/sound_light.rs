use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SoundLightError {
    #[error("SOUND_LIGHT_JSON")]
    Json,
    #[error("SOUND_LIGHT_RANGE")]
    Range,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundLightConfig {
    #[serde(rename = "Sound")]
    pub sound: SoundBlock,
    #[serde(rename = "Light")]
    pub light: LightBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundBlock {
    #[serde(rename = "Volume")]
    pub volume: u8,
    #[serde(rename = "AgentPrompt")]
    pub agent_prompt: SoundPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundPrompt {
    #[serde(rename = "Enabled")]
    pub enabled: bool,
    #[serde(rename = "Preview", default, skip_serializing_if = "is_false")]
    pub preview: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightBlock {
    #[serde(rename = "Brightness")]
    pub brightness: u8,
    #[serde(rename = "AgentPrompt")]
    pub agent_prompt: LightPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightPrompt {
    #[serde(rename = "Enabled")]
    pub enabled: bool,
    #[serde(rename = "Color")]
    pub color: Rgb,
    #[serde(rename = "DurationMs")]
    pub duration_ms: u32,
    #[serde(rename = "Preview", default, skip_serializing_if = "is_false")]
    pub preview: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    #[serde(rename = "R")]
    pub r: u8,
    #[serde(rename = "G")]
    pub g: u8,
    #[serde(rename = "B")]
    pub b: u8,
}

fn is_false(v: &bool) -> bool {
    !*v
}

impl SoundLightConfig {
    /// 协议文档默认值。
    pub fn protocol_default() -> Self {
        Self {
            sound: SoundBlock {
                volume: 80,
                agent_prompt: SoundPrompt {
                    enabled: true,
                    preview: false,
                },
            },
            light: LightBlock {
                brightness: 80,
                agent_prompt: LightPrompt {
                    enabled: true,
                    color: Rgb {
                        r: 255,
                        g: 128,
                        b: 0,
                    },
                    duration_ms: 6000,
                    preview: false,
                },
            },
        }
    }

    pub fn parse(json: &str) -> Result<Self, SoundLightError> {
        let cfg: Self = serde_json::from_str(json).map_err(|_| SoundLightError::Json)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn to_json(&self) -> Result<String, SoundLightError> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|_| SoundLightError::Json)
    }

    pub fn validate(&self) -> Result<(), SoundLightError> {
        if self.sound.volume > 100 || self.light.brightness > 100 {
            return Err(SoundLightError::Range);
        }
        if self.light.agent_prompt.duration_ms > 60_000 {
            return Err(SoundLightError::Range);
        }
        Ok(())
    }

    pub fn with_preview(mut self, sound: bool, light: bool) -> Self {
        self.sound.agent_prompt.preview = sound;
        self.light.agent_prompt.preview = light;
        self
    }
}

/// `%AppData%\Roaming\Ulanzi\D50\d50-sound-light.json` 的相对尾路径（调用方拼 AppData）。
pub const SOUND_LIGHT_RELATIVE: &str = r"Ulanzi\D50\d50-sound-light.json";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trips() {
        let cfg = SoundLightConfig::protocol_default();
        let json = cfg.to_json().unwrap();
        let parsed = SoundLightConfig::parse(&json).unwrap();
        assert_eq!(parsed.sound.volume, 80);
        assert_eq!(parsed.light.agent_prompt.color.r, 255);
    }

    #[test]
    fn rejects_volume_over_100() {
        let mut cfg = SoundLightConfig::protocol_default();
        cfg.sound.volume = 101;
        assert_eq!(cfg.validate(), Err(SoundLightError::Range));
    }
}
