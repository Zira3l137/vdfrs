//! What a file in an archive is, judged by its extension, and how the explorer shows it.

use gpui_kit::App;
use gpui_kit::Hsla;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    Folder,
    Texture,
    Image,
    Mesh,
    Model,
    Morph,
    Animation,
    ModelScript,
    World,
    Sound,
    Music,
    Video,
    Script,
    Cutscene,
    Font,
    Text,
    Archive,
    Other,
}

impl FileKind {
    pub fn of(name: &str) -> Self {
        let ext = name.rsplit_once('.').map_or("", |(_, ext)| ext).to_ascii_uppercase();
        match ext.as_str() {
            "TEX" => Self::Texture,
            "TGA" | "BMP" | "PNG" | "JPG" | "JPEG" | "DDS" => Self::Image,
            "MRM" | "MSH" | "3DS" | "ASC" => Self::Mesh,
            "MDH" | "MDM" | "MDL" => Self::Model,
            "MMB" | "MMS" => Self::Morph,
            "MAN" => Self::Animation,
            "MDS" | "MSB" => Self::ModelScript,
            "ZEN" => Self::World,
            "WAV" | "OGG" | "MP3" => Self::Sound,
            "SGT" | "STY" | "DLS" | "MID" => Self::Music,
            "BIK" | "AVI" => Self::Video,
            "D" | "SRC" | "DAT" => Self::Script,
            "CSL" | "BIN" => Self::Cutscene,
            "FNT" => Self::Font,
            "TXT" | "INI" | "LOG" | "MD" | "YML" | "CFG" | "PFX" | "XML" | "JSON" => Self::Text,
            "VDF" | "MOD" | "ZIP" => Self::Archive,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Folder => "Folder",
            Self::Texture => "Texture",
            Self::Image => "Image",
            Self::Mesh => "Mesh",
            Self::Model => "Model",
            Self::Morph => "Morph mesh",
            Self::Animation => "Animation",
            Self::ModelScript => "Model script",
            Self::World => "World",
            Self::Sound => "Sound",
            Self::Music => "Music",
            Self::Video => "Video",
            Self::Script => "Script",
            Self::Cutscene => "Cutscene",
            Self::Font => "Font",
            Self::Text => "Text",
            Self::Archive => "Archive",
            Self::Other => "File",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Folder => IconName::Folder,
            Self::Texture | Self::Image => IconName::FileImage,
            Self::Mesh => IconName::Box,
            Self::Model => IconName::PersonStanding,
            Self::Morph => IconName::Drama,
            Self::Animation => IconName::Footprints,
            Self::ModelScript => IconName::ScrollText,
            Self::World => IconName::Earth,
            Self::Sound => IconName::AudioLines,
            Self::Music => IconName::Music,
            Self::Video => IconName::Film,
            Self::Script => IconName::FileCode,
            Self::Cutscene => IconName::MessageSquareText,
            Self::Font => IconName::Type,
            Self::Text => IconName::FileText,
            Self::Archive => IconName::Package,
            Self::Other => IconName::File,
        }
    }

    pub fn color(self, cx: &App) -> Hsla {
        let theme = cx.theme();
        match self {
            Self::Folder => theme.yellow,
            Self::Texture | Self::Image => theme.magenta,
            Self::Mesh | Self::Model | Self::Morph => theme.cyan,
            Self::Animation | Self::ModelScript => theme.blue,
            Self::World => theme.green,
            Self::Sound | Self::Music | Self::Video => theme.red,
            Self::Script | Self::Cutscene => theme.blue_light,
            Self::Font | Self::Text | Self::Archive | Self::Other => theme.muted_foreground,
        }
    }
}
