//! Hızlı geçiş (Ctrl+K benzeri) için bulanık alt dizi eşleştirici. Sadece std.
//!
//! Sorgunun karakterleri adayda sırayla (araya başka karakterler girebilir) geçmelidir.
//! Eşleşme büyük/küçük harf duyarsızdır; Türkçe'de `ı`, `i` ve `İ` birbirine eşittir.
//! Eşleştirme açgözlüdür: her sorgu karakteri, kalan en erken konumda aranır.

use std::cmp::Reverse;

/// Ayırıcı karakterler: bu karakterden sonra gelen harf "kelime başı" sayılır.
fn is_separator(c: char) -> bool {
    matches!(c, ' ' | '-' | '_' | '#' | '@' | '/' | '.')
}

/// Karşılaştırma için katlanmış karakter dizisi. Karakter bazlıdır (bayt dilimi yok),
/// bu yüzden emoji ve çok baytlı harfler paniğe yol açmaz.
fn fold(s: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        for lower in c.to_lowercase() {
            match lower {
                // 'İ' küçültülünce "i" + birleştirici nokta çıkar; nokta atılır.
                '\u{307}' => {}
                // Noktasız 'ı', noktalı 'i' ile aynı sayılır.
                'ı' => out.push('i'),
                other => out.push(other),
            }
        }
    }
    out
}

/// `query`, `candidate` içinde bulanık eşleşiyorsa puanı döner; eşleşmezse `None`.
/// Daha yüksek puan daha iyidir. Boş sorgu her adayla `Some(0)` verir.
///
/// Puanlama:
/// - eşleşen her karakter +10
/// - adayın başında veya ayırıcıdan sonra eşleşme +15 (kelime başı)
/// - bir önceki eşleşmenin hemen ardından gelen eşleşme +5
/// - ilk ve son eşleşme arasında eşleşmeyen her karakter -1
/// - aday sorguyla başlıyorsa +30
/// - sorgu adayda bitişik bir alt dizi olarak geçiyorsa +20
pub fn score(query: &str, candidate: &str) -> Option<i32> {
    let q = fold(query);
    if q.is_empty() {
        return Some(0);
    }
    let c = fold(candidate);

    let mut score = 0i32;
    let mut from = 0usize;
    let mut prev: Option<usize> = None;
    let mut first = 0usize;
    let mut last = 0usize;
    for (k, &qc) in q.iter().enumerate() {
        let p = from + c[from..].iter().position(|&cc| cc == qc)?;
        score += 10;
        if p == 0 || is_separator(c[p - 1]) {
            score += 15;
        }
        if prev.is_some_and(|pp| pp + 1 == p) {
            score += 5;
        }
        if k == 0 {
            first = p;
        }
        prev = Some(p);
        last = p;
        from = p + 1;
    }

    // `span >= q.len()` çünkü her sorgu karakteri farklı bir konuma denk gelir.
    let gap = (last - first + 1) - q.len();
    score -= i32::try_from(gap).unwrap_or(i32::MAX);
    if c.starts_with(&q) {
        score += 30;
    }
    if c.windows(q.len()).any(|w| w == q.as_slice()) {
        score += 20;
    }
    Some(score)
}

/// `candidates` içinden `query` ile eşleşenlerin indekslerini döner. Sıra: puan azalan,
/// eşitlikte önce daha kısa aday (karakter sayısı), sonra özgün indeks. En fazla `limit` tane.
pub fn rank<S: AsRef<str>>(query: &str, candidates: &[S], limit: usize) -> Vec<usize> {
    let mut matched: Vec<(i32, usize, usize)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(i, cand)| {
            let s = cand.as_ref();
            score(query, s).map(|sc| (sc, s.chars().count(), i))
        })
        .collect();
    matched.sort_unstable_by_key(|&(sc, len, i)| (Reverse(sc), len, i));
    matched.into_iter().take(limit).map(|(_, _, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_subsequence_matches() {
        assert!(score("gc", "genel-chat").is_some());
        assert!(score("gnc", "genel-chat").is_some());
        assert_eq!(score("xz", "genel"), None);
    }

    #[test]
    fn order_matters() {
        assert_eq!(score("ba", "ab"), None);
    }

    #[test]
    fn query_longer_than_candidate_does_not_match() {
        assert_eq!(score("abcd", "ab"), None);
    }

    #[test]
    fn case_insensitive() {
        assert!(score("GEN", "genel").is_some());
        assert!(score("gen", "GENEL").is_some());
        assert_eq!(score("GEN", "genel"), score("gen", "Genel"));
    }

    #[test]
    fn turkish_dotted_and_dotless_i() {
        assert!(score("istanbul", "İstanbul").is_some());
        assert!(score("İSTANBUL", "istanbul").is_some());
        assert!(score("ıstanbul", "istanbul").is_some());
        assert!(score("istanbul", "ıstanbul").is_some());
        assert!(score("ı", "İ").is_some());
        assert!(score("ı", "I").is_some());
    }

    #[test]
    fn turkish_c_s_letters() {
        assert!(score("çay", "Çaylar").is_some());
        assert!(score("ÇAY", "çaylık").is_some());
        assert!(score("şişli", "ŞİŞLİ").is_some());
        assert!(score("şiş", "Şişli").is_some());
        // ç ile c, ş ile s ayrı harflerdir; yalnızca ı/i/İ katlanır.
        assert_eq!(score("cay", "çay"), None);
        assert_eq!(score("sisli", "şişli"), None);
    }

    #[test]
    fn prefix_beats_substring_beats_scattered() {
        let prefix = score("ab", "abc").unwrap();
        let substring = score("ab", "xab").unwrap();
        let scattered = score("ab", "axxb").unwrap();
        assert!(prefix > substring, "{prefix} > {substring}");
        assert!(substring > scattered, "{substring} > {scattered}");
    }

    #[test]
    fn word_start_bonus() {
        // "gs": iki adayda da g ve s kelime başında, ama "genel-sohbet"te aralık daha kısa.
        let good = score("gs", "genel-sohbet").unwrap();
        let bad = score("gs", "gizli-oda-s").unwrap();
        assert!(good > bad, "{good} > {bad}");
        assert_eq!(rank("gs", &["gizli-oda-s", "genel-sohbet"], 10), vec![1, 0]);
    }

    #[test]
    fn word_start_beats_mid_word() {
        let word_start = score("ab", "x-ab").unwrap();
        let mid_word = score("ab", "xxab").unwrap();
        assert!(word_start > mid_word, "{word_start} > {mid_word}");
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(score("", "anything"), Some(0));
        assert_eq!(score("", ""), Some(0));
        assert_eq!(rank("", &["bb", "a", "cc"], 10), vec![1, 0, 2]);
    }

    #[test]
    fn rank_skips_non_matches_and_respects_limit() {
        assert_eq!(rank("z", &["abc", "zed"], 5), vec![1]);
        assert_eq!(rank("a", &["a", "ab", "abc"], 2).len(), 2);
        assert!(rank("a", &["a", "ab"], 0).is_empty());
    }

    #[test]
    fn rank_ties_go_to_shorter_then_original_index() {
        // İkisi de aynı puanda; kısa olan önce gelir, indeksi büyük olsa bile.
        assert_eq!(rank("a", &["xya", "xa"], 10), vec![1, 0]);
        // Aynı puan ve aynı uzunluk: özgün indeks sırası.
        assert_eq!(rank("x", &["ax", "bx"], 10), vec![0, 1]);
    }

    #[test]
    fn emoji_and_multibyte_do_not_panic() {
        assert!(score("a", "🎮 announce").is_some());
        assert!(score("🎮", "🎮 oyun").is_some());
        assert_eq!(score("🎮", "oyun"), None);
        assert_eq!(rank("🎮", &["🎮 oyun", "oyun", "🙂🎮"], 5), vec![0, 2]);
        assert!(score("ğ", "Ğ🙂ğ").is_some());
    }
}
