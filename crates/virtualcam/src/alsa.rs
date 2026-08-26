#[cfg_attr(alsa, path = "alsa/supported.rs")]
#[cfg_attr(not(alsa), path = "alsa/unsupported.rs")]
mod device;

pub use device::Device;

const CARDS: &str = "/proc/asound/cards";
const LOOPBACK_CARD: &str = "Loopback";

pub fn available() -> bool {
    cfg!(alsa)
}

pub fn loopback_devices() -> Vec<(String, String)> {
    if !available() {
        return Vec::new();
    }
    let Ok(cards) = std::fs::read_to_string(CARDS) else {
        return Vec::new();
    };
    loopback_cards(&cards)
        .into_iter()
        .map(|(id, name)| (format!("plughw:CARD={id},DEV=0"), name))
        .collect()
}

fn loopback_cards(cards: &str) -> Vec<(String, String)> {
    cards
        .lines()
        .filter_map(card)
        .filter(|(id, _)| id.starts_with(LOOPBACK_CARD))
        .collect()
}

fn card(line: &str) -> Option<(String, String)> {
    let (index, rest) = line.split_once('[')?;
    index.trim().parse::<u32>().ok()?;
    let (id, rest) = rest.split_once(']')?;
    let name = rest.split_once(':')?.1;
    Some((id.trim().to_string(), name.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARDS: &str = concat!(
        " 0 [PCH            ]: HDA-Intel - HDA Intel PCH\n",
        "                      HDA Intel PCH at 0xf7f10000 irq 33\n",
        " 1 [Loopback       ]: Loopback - Loopback\n",
        "                      Loopback 1\n",
    );

    #[test]
    fn only_the_loopback_cards_are_kept() {
        assert_eq!(
            loopback_cards(CARDS),
            vec![(String::from("Loopback"), String::from("Loopback - Loopback"))]
        );
    }

    #[test]
    fn the_lines_of_details_are_not_cards() {
        assert_eq!(card("                      Loopback 1"), None);
        assert_eq!(
            card(" 0 [PCH            ]: HDA-Intel - HDA Intel PCH").unwrap().0,
            "PCH"
        );
    }

    #[test]
    fn no_cards_at_all_is_no_devices() {
        assert!(loopback_cards("--- no soundcards ---\n").is_empty());
    }
}
