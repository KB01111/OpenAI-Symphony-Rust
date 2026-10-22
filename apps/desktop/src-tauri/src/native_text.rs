//! User-visible text owned by the native host (tray menu, notifications) in the interface
//! language. The webview translates its own strings; everything the host shows itself comes from
//! here, so the Swedish and English experiences agree.

/// Interface languages the host can speak. Mirrors `Locale` in `@symphony/shared-types`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeLocale {
    #[default]
    En,
    Sv,
}

impl NativeLocale {
    /// Exactly `en` or `sv`; anything else is rejected so IPC cannot smuggle in other values.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "en" => Some(Self::En),
            "sv" => Some(Self::Sv),
            _ => None,
        }
    }

    /// A locale read from stored settings; an unknown value falls back to English.
    pub fn from_setting(value: &str) -> Self {
        Self::parse(value).unwrap_or_default()
    }
}

/// Labels of the system tray icon and its menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayText {
    pub show: &'static str,
    pub pause: &'static str,
    pub resume: &'static str,
    pub quit: &'static str,
    pub tooltip: &'static str,
}

pub fn tray_text(locale: NativeLocale) -> TrayText {
    match locale {
        NativeLocale::En => TrayText {
            show: "Open Symphony Desktop",
            pause: "Pause task dispatching",
            resume: "Resume task dispatching",
            quit: "Stop runtime and exit",
            tooltip: "Symphony Desktop",
        },
        NativeLocale::Sv => TrayText {
            show: "Öppna Symphony Desktop",
            pause: "Pausa uppgiftsutskick",
            resume: "Återuppta uppgiftsutskick",
            quit: "Stoppa körtiden och avsluta",
            tooltip: "Symphony Desktop",
        },
    }
}

/// Title of the notification for an orchestrator event, or `None` when the event is not one an
/// operator needs to be told about while the window is in the background.
pub fn notification_title(locale: NativeLocale, event_type: &str) -> Option<&'static str> {
    Some(match (locale, event_type) {
        (NativeLocale::En, "approval/requested") => "Approval required",
        (NativeLocale::En, "attempt/succeeded") => "Ready for review",
        (NativeLocale::En, "attempt/failed") => "Agent run failed",
        (NativeLocale::Sv, "approval/requested") => "Godkännande krävs",
        (NativeLocale::Sv, "attempt/succeeded") => "Redo för granskning",
        (NativeLocale::Sv, "attempt/failed") => "Agentkörningen misslyckades",
        _ => return None,
    })
}

/// Body of the notification for an orchestrator event. English keeps the orchestrator's own text.
/// Swedish translates its sentence and keeps the details that are in it: the requested action of an
/// approval, the Merge Readiness score of a finished run and the error of a failed one. When the
/// message does not have the expected shape the original text is appended instead, so a detail is
/// never dropped.
pub fn notification_body(
    locale: NativeLocale,
    event_type: &str,
    identifier: Option<&str>,
    message: &str,
) -> String {
    let lead = match (locale, event_type) {
        (NativeLocale::Sv, "approval/requested") => {
            // Codex titles already start with the phrase the orchestrator prepends: drop every copy.
            let mut detail = message;
            while let Some(rest) = detail.strip_prefix("Approval requested: ") {
                detail = rest;
            }
            format!("väntar på ditt godkännande: {detail}")
        }
        (NativeLocale::Sv, "attempt/succeeded") => match merge_readiness(message) {
            Some(score) => format!("klar för granskning (sammanslagningsberedskap {score})"),
            None => format!("klar för granskning ({message})"),
        },
        (NativeLocale::Sv, "attempt/failed") => {
            let detail = message.split_once(" failed: ").map_or(message, |(_, e)| e);
            format!("körningen misslyckades: {detail}")
        }
        _ => message.to_string(),
    };
    match identifier {
        Some(id) => format!("{id}: {lead}"),
        None => {
            let mut chars = lead.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => lead,
            }
        }
    }
}

/// The `n/5` of "... (Merge Readiness: n/5). ...".
fn merge_readiness(message: &str) -> Option<&str> {
    let (_, rest) = message.split_once("Merge Readiness: ")?;
    let score = rest.split(')').next()?.trim();
    (!score.is_empty()).then_some(score)
}

/// Title and body of the notification shown when `runtime.json` could not be published.
pub fn discovery_unavailable(locale: NativeLocale) -> (&'static str, &'static str) {
    match locale {
        NativeLocale::En => (
            "Symphony runtime discovery unavailable",
            "CLI tools cannot attach to this runtime. Open Settings > Application for details.",
        ),
        NativeLocale::Sv => (
            "Körtidens identifiering är inte tillgänglig",
            "CLI-verktyg kan inte ansluta till den här körtiden. Öppna Inställningar > Program för detaljer.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_locales_parse() {
        assert_eq!(NativeLocale::parse("en"), Some(NativeLocale::En));
        assert_eq!(NativeLocale::parse("sv"), Some(NativeLocale::Sv));
        for bad in ["", "EN", "sv-SE", "de", " en", "en\0"] {
            assert_eq!(NativeLocale::parse(bad), None, "{bad:?}");
        }
        assert_eq!(NativeLocale::from_setting("fr"), NativeLocale::En);
    }

    #[test]
    fn every_tray_label_is_translated_and_non_empty() {
        let (en, sv) = (tray_text(NativeLocale::En), tray_text(NativeLocale::Sv));
        for (a, b) in [
            (en.show, sv.show),
            (en.pause, sv.pause),
            (en.resume, sv.resume),
            (en.quit, sv.quit),
        ] {
            assert!(!a.is_empty() && !b.is_empty());
            assert_ne!(a, b, "{a} has no Swedish translation");
        }
    }

    #[test]
    fn english_notification_body_is_the_orchestrators_text() {
        assert_eq!(
            notification_body(
                NativeLocale::En,
                "attempt/failed",
                Some("ABC-1"),
                "Task failed"
            ),
            "ABC-1: Task failed"
        );
        assert_eq!(
            notification_body(NativeLocale::En, "attempt/failed", None, "Task failed"),
            "Task failed"
        );
    }

    #[test]
    fn swedish_notification_body_is_translated_and_keeps_the_details() {
        let sv = NativeLocale::Sv;
        assert_eq!(
            notification_body(
                sv,
                "approval/requested",
                Some("ABC-1"),
                "Approval requested: command_execution"
            ),
            "ABC-1: väntar på ditt godkännande: command_execution"
        );
        // Codex approval titles already begin with the orchestrator's phrase.
        assert_eq!(
            notification_body(
                sv,
                "approval/requested",
                Some("ABC-1"),
                "Approval requested: Approval requested: command_execution"
            ),
            "ABC-1: väntar på ditt godkännande: command_execution"
        );
        assert_eq!(
            notification_body(
                sv,
                "approval/requested",
                None,
                "Approval requested: Run validation command"
            ),
            "Väntar på ditt godkännande: Run validation command"
        );
        assert_eq!(
            notification_body(
                sv,
                "attempt/succeeded",
                Some("ABC-1"),
                "Completed execution for ABC-1 (Merge Readiness: 2/5). Ready for human review."
            ),
            "ABC-1: klar för granskning (sammanslagningsberedskap 2/5)"
        );
        assert_eq!(
            notification_body(
                sv,
                "attempt/failed",
                Some("ABC-1"),
                "Task ABC-1 failed: exit code 2"
            ),
            "ABC-1: körningen misslyckades: exit code 2"
        );
        assert_eq!(
            notification_body(sv, "attempt/succeeded", None, "unexpected text"),
            "Klar för granskning (unexpected text)"
        );
    }

    #[test]
    fn notifications_cover_the_same_events_in_both_languages() {
        for event in ["approval/requested", "attempt/succeeded", "attempt/failed"] {
            let en = notification_title(NativeLocale::En, event).expect(event);
            let sv = notification_title(NativeLocale::Sv, event).expect(event);
            assert_ne!(en, sv);
        }
        assert_eq!(notification_title(NativeLocale::Sv, "task/created"), None);
        assert_eq!(notification_title(NativeLocale::En, "task/created"), None);
    }
}
