pub const ALPHABET: &str = "sadfjklewcmpgh";

pub fn make(n: usize) -> Vec<String> {
    let mut all = vec![String::new()];
    let mut used = 0;
    while all.len() - used < n || all.len() == 1 {
        let stem = all[used].clone();
        used += 1;
        all.extend(ALPHABET.chars().map(|c| format!("{stem}{c}")));
    }
    all.drain(used..used + n).collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    Pick(usize),
    Miss,
}

#[derive(Debug, Default)]
pub struct Labels {
    pub all: Vec<String>,
    pub typed: String,
}

impl Labels {
    pub fn new(n: usize) -> Labels {
        Labels { all: make(n), typed: String::new() }
    }

    pub fn push(&mut self, c: char) -> Step {
        let typed = format!("{}{c}", self.typed);
        if let Some(i) = self.all.iter().position(|l| *l == typed) {
            return Step::Pick(i);
        }
        if !self.all.iter().any(|l| l.starts_with(&typed)) {
            return Step::Miss;
        }
        self.typed = typed;
        Step::Wait
    }

    pub fn pop(&mut self) {
        self.typed.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn few_targets_get_one_letter_each() {
        assert_eq!(make(3), ["s", "a", "d"]);
    }

    #[test]
    fn labels_are_unique_and_none_starts_another() {
        for n in [0, 1, 13, 14, 15, 40, 196, 300] {
            let all = make(n);
            assert_eq!(all.len(), n);
            for (i, a) in all.iter().enumerate() {
                for (j, b) in all.iter().enumerate() {
                    assert!(i == j || !b.starts_with(a.as_str()), "{n}: {a} starts {b}");
                }
            }
        }
    }

    #[test]
    fn labels_stay_short() {
        assert!(make(14).iter().all(|l| l.len() == 1));
        assert!(make(196).iter().all(|l| l.len() <= 2));
        assert!(make(15).iter().any(|l| l.len() == 1));
    }

    #[test]
    fn labels_use_only_the_alphabet() {
        assert!(make(300).iter().flat_map(|l| l.chars()).all(|c| ALPHABET.contains(c)));
    }

    #[test]
    fn typing_a_whole_label_picks_it() {
        let mut l = Labels::new(20);
        let target = l.all.iter().position(|s| s.len() == 2).unwrap();
        let label = l.all[target].clone();
        let mut chars = label.chars();
        assert_eq!(l.push(chars.next().unwrap()), Step::Wait);
        assert_eq!(l.typed.len(), 1);
        assert_eq!(l.push(chars.next().unwrap()), Step::Pick(target));
    }

    #[test]
    fn a_letter_no_label_has_is_dropped() {
        let mut l = Labels::new(3);
        assert_eq!(l.push('z'), Step::Miss);
        assert_eq!(l.typed, "");
        assert_eq!(l.push('a'), Step::Pick(1));
    }

    #[test]
    fn backspace_takes_back_a_letter() {
        let mut l = Labels::new(20);
        let first = l.all.iter().find(|s| s.len() == 2).unwrap().chars().next().unwrap();
        l.push(first);
        l.pop();
        assert_eq!(l.typed, "");
    }
}
