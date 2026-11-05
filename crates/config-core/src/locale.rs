/// 余额条目标签语言；其它系统语言回退到英文。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    ZhHans,
    En,
}

impl Locale {
    /// 从 BCP-47 / Windows 语言名粗判；无法识别则 `En`。
    pub fn from_tag(tag: &str) -> Self {
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("zh") {
            Self::ZhHans
        } else {
            Self::En
        }
    }
}

pub mod labels {
    use super::Locale;

    pub fn session_remaining(locale: Locale) -> &'static str {
        match locale {
            Locale::ZhHans => "5小时剩余",
            Locale::En => "5h left",
        }
    }

    pub fn week_remaining(locale: Locale) -> &'static str {
        match locale {
            Locale::ZhHans => "周剩余",
            Locale::En => "Weekly left",
        }
    }

    pub fn no_data(locale: Locale) -> &'static str {
        match locale {
            Locale::ZhHans => "暂无数据",
            Locale::En => "No data",
        }
    }

    pub fn available_balance(locale: Locale) -> &'static str {
        match locale {
            Locale::ZhHans => "可用余额",
            Locale::En => "Balance",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zh_tags_select_hans() {
        assert_eq!(Locale::from_tag("zh-CN"), Locale::ZhHans);
        assert_eq!(Locale::from_tag("zh-Hans-CN"), Locale::ZhHans);
    }

    #[test]
    fn other_tags_fall_back_to_en() {
        assert_eq!(Locale::from_tag("en-US"), Locale::En);
        assert_eq!(Locale::from_tag("ja-JP"), Locale::En);
    }
}
