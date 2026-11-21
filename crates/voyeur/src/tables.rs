// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Static tables the Shell's built-in providers search: settings pages and
//! emoji. Pure data; the Shell decides what choosing one does.

/// A settings page: its name and the words it is also found by.
#[derive(Clone, Copy, Debug)]
pub struct SettingPage { pub name: &'static str, pub keywords: &'static str }

/// Settings pages by name, the way GNOME Settings and KDE System Settings
/// find a page from its name or a word on it.
pub const SETTINGS: &[SettingPage] = &[
    SettingPage { name: "Display", keywords: "screen monitor resolution scale brightness" },
    SettingPage { name: "Sound", keywords: "volume audio speakers microphone output input" },
    SettingPage { name: "Wi-Fi", keywords: "wifi wireless network internet" },
    SettingPage { name: "Network", keywords: "ethernet internet ip dns proxy vpn wireguard" },
    SettingPage { name: "Bluetooth", keywords: "wireless headphones pairing" },
    SettingPage { name: "Keyboard", keywords: "layout shortcuts keys typing input method" },
    SettingPage { name: "Mouse and Touchpad", keywords: "pointer cursor scroll trackpad" },
    SettingPage { name: "Appearance", keywords: "theme dark light accent colors look" },
    SettingPage { name: "Wallpaper", keywords: "background desktop picture" },
    SettingPage { name: "Notifications", keywords: "bell alerts do not disturb" },
    SettingPage { name: "Power", keywords: "battery sleep suspend energy" },
    SettingPage { name: "Date and Time", keywords: "clock timezone calendar" },
    SettingPage { name: "Language and Region", keywords: "locale formats translation" },
    SettingPage { name: "Accessibility", keywords: "zoom contrast screen reader large text" },
    SettingPage { name: "Privacy", keywords: "collar permissions security" },
    SettingPage { name: "Users", keywords: "accounts login password" },
    SettingPage { name: "Storage", keywords: "disks drives space sexdrive" },
    SettingPage { name: "Updates", keywords: "upgrade packages sexshop" },
    SettingPage { name: "Default Apps", keywords: "open with associations" },
    SettingPage { name: "Workspaces", keywords: "atlas desktops virtual" },
    SettingPage { name: "About This Computer", keywords: "system version hardware info" },
];

/// An emoji: its name, the words it is also found by, and the character.
#[derive(Clone, Copy, Debug)]
pub struct Emoji { pub name: &'static str, pub keywords: &'static str, pub text: &'static str }

pub const EMOJI: &[Emoji] = &[
    Emoji { name: "grinning face", keywords: "smile happy", text: "\u{1F600}" },
    Emoji { name: "face with tears of joy", keywords: "laugh lol", text: "\u{1F602}" },
    Emoji { name: "smiling face with heart eyes", keywords: "love crush", text: "\u{1F60D}" },
    Emoji { name: "winking face", keywords: "wink flirt", text: "\u{1F609}" },
    Emoji { name: "thinking face", keywords: "hmm ponder", text: "\u{1F914}" },
    Emoji { name: "crying face", keywords: "sad tear", text: "\u{1F622}" },
    Emoji { name: "angry face", keywords: "mad", text: "\u{1F620}" },
    Emoji { name: "thumbs up", keywords: "yes ok approve like", text: "\u{1F44D}" },
    Emoji { name: "thumbs down", keywords: "no dislike", text: "\u{1F44E}" },
    Emoji { name: "clapping hands", keywords: "applause bravo", text: "\u{1F44F}" },
    Emoji { name: "waving hand", keywords: "hello hi bye", text: "\u{1F44B}" },
    Emoji { name: "folded hands", keywords: "please thanks pray", text: "\u{1F64F}" },
    Emoji { name: "red heart", keywords: "love", text: "\u{2764}\u{FE0F}" },
    Emoji { name: "fire", keywords: "hot lit flame", text: "\u{1F525}" },
    Emoji { name: "sparkles", keywords: "shiny new magic", text: "\u{2728}" },
    Emoji { name: "party popper", keywords: "celebrate tada", text: "\u{1F389}" },
    Emoji { name: "rocket", keywords: "launch ship fast", text: "\u{1F680}" },
    Emoji { name: "bell", keywords: "notification ring", text: "\u{1F514}" },
    Emoji { name: "eyes", keywords: "look watch voyeur", text: "\u{1F440}" },
    Emoji { name: "check mark", keywords: "done yes tick", text: "\u{2705}" },
    Emoji { name: "cross mark", keywords: "no wrong x", text: "\u{274C}" },
    Emoji { name: "warning", keywords: "caution alert", text: "\u{26A0}\u{FE0F}" },
    Emoji { name: "hundred points", keywords: "100 perfect", text: "\u{1F4AF}" },
    Emoji { name: "light bulb", keywords: "idea", text: "\u{1F4A1}" },
    Emoji { name: "bug", keywords: "insect error", text: "\u{1F41B}" },
    Emoji { name: "spider web", keywords: "cobweb", text: "\u{1F578}\u{FE0F}" },
    Emoji { name: "cat face", keywords: "kitten pet", text: "\u{1F431}" },
    Emoji { name: "dog face", keywords: "puppy pet", text: "\u{1F436}" },
    Emoji { name: "pizza", keywords: "food", text: "\u{1F355}" },
    Emoji { name: "hot beverage", keywords: "coffee tea", text: "\u{2615}" },
    Emoji { name: "kiss mark", keywords: "lips love", text: "\u{1F48B}" },
    Emoji { name: "peach", keywords: "fruit butt", text: "\u{1F351}" },
    Emoji { name: "eggplant", keywords: "aubergine vegetable", text: "\u{1F346}" },
    Emoji { name: "rainbow", keywords: "colors pride", text: "\u{1F308}" },
    Emoji { name: "globe", keywords: "world earth web", text: "\u{1F310}" },
    Emoji { name: "lock", keywords: "secure private", text: "\u{1F512}" },
    Emoji { name: "key", keywords: "password unlock", text: "\u{1F511}" },
    Emoji { name: "magnifying glass", keywords: "search find", text: "\u{1F50D}" },
];
