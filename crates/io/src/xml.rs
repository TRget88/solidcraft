//! Reading the attributes of an XML tag, shared by the 3MF, E57 and SVG readers.

use std::collections::HashSet;

use quick_xml::events::BytesStart;

/// The attributes of tag `e` in the order they are written, each as its local name and its value,
/// collected into a list or into a map by local name. The value has its entities and white space
/// normalised, or stays as written when that fails. A malformed attribute is left out. Of a name
/// written more than once (prefix and all) only the first copy that reads is kept, and each later
/// copy is skipped whole.
///
/// quick-xml's own check for a repeated name compares each repeat with every name before it, so a
/// tag that repeats one name many times took time in the square of its length (#21468). Here that
/// check is off and the names already read are looked up in a short list, or in a set once there
/// are many, so the time grows with the tag. This keeps what the check kept, except on two kinds of
/// malformed tag. The check took a name as seen before reading its value, so a first copy with an
/// unquoted value (`r=5 r="7"`) hid every later copy. And it skipped a repeat only up to the next
/// white space, so an attribute written right after a repeat with no space between was lost, and
/// the tail of a repeated value with a space in it could be read as more attributes.
pub(crate) fn attributes<C: FromIterator<(String, String)>>(e: &BytesStart) -> C {
    let mut all = e.attributes();
    all.with_checks(false);
    let mut seen = Seen::default();
    all.flatten()
        .filter(|a| seen.first(a.key.into_inner()))
        .map(|a| {
            let k = String::from_utf8_lossy(a.key.local_name().as_ref()).to_string();
            let v = a
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(|v| v.to_string())
                .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).to_string());
            (k, v)
        })
        .collect()
}

/// Up to this many names are looked up in a list, which is quicker than hashing for the few names
/// an ordinary tag has. Past it they go in a set. quick-xml's check made the same split.
const FEW: usize = 32;

/// The full names of a tag's attributes read so far.
#[derive(Default)]
struct Seen<'a> {
    few: Vec<&'a [u8]>,
    many: Option<HashSet<&'a [u8]>>,
}

impl<'a> Seen<'a> {
    /// Whether `name` is read here for the first time. It is remembered from then on.
    fn first(&mut self, name: &'a [u8]) -> bool {
        if let Some(many) = &mut self.many {
            return many.insert(name);
        }
        if self.few.contains(&name) {
            return false;
        }
        if self.few.len() < FEW {
            self.few.push(name);
        } else {
            let mut many: HashSet<&[u8]> = self.few.drain(..).collect();
            many.insert(name);
            self.many = Some(many);
        }
        true
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use quick_xml::events::{BytesStart, Event};

    /// The attributes of tag `e` read with quick-xml's own check for a repeated name, the way the
    /// readers read them before #21468, to compare [`super::attributes`] with.
    pub(crate) fn checked(e: &BytesStart) -> Vec<(String, String)> {
        e.attributes()
            .flatten()
            .map(|a| {
                let k = String::from_utf8_lossy(a.key.local_name().as_ref()).to_string();
                let v = a
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).to_string());
                (k, v)
            })
            .collect()
    }

    /// Every start and empty tag of `xml`, in order.
    pub(crate) fn tags(xml: &str) -> Vec<BytesStart<'static>> {
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut out = Vec::new();
        loop {
            match reader.read_event() {
                Ok(Event::Start(e) | Event::Empty(e)) => out.push(e.into_owned()),
                Ok(Event::Eof) | Err(_) => return out,
                Ok(_) => {}
            }
        }
    }

    /// How many names the readers' tests write once and then repeat, enough that quick-xml's check
    /// took seconds where reading the tag now takes milliseconds.
    pub(crate) const MANY: usize = 50_000;

    /// Attributes for a tag that names `n` attributes once each and then repeats the last name
    /// `n` times, all with empty values. The names are all as long, so quick-xml's check compared
    /// each repeat with every name before it, `n` times `n` comparisons.
    pub(crate) fn repeated_names(n: usize) -> String {
        let mut out = String::with_capacity(2 * n * 11);
        for i in 0..n {
            out.push_str(&format!(" a{i:06}=\"\""));
        }
        out.push_str(&format!(" a{:06}=\"\"", n.saturating_sub(1)).repeat(n));
        out
    }

    /// Tags of each kind of attribute the readers meet, with names repeated a few times and many
    /// times, read as before.
    #[test]
    fn attributes_are_read_as_with_the_check() {
        let many = repeated_names(40);
        let xml =
            format!(r#"<r xmlns="u" xmlns:p="v"><a x="1" y='2' p:x="3" x="4" z="a &amp; b&#10;c" w="&bad;" bad y="5"/><b{many} a000001="6"/></r>"#);
        let tags = tags(&xml);
        assert_eq!(tags.len(), 3);
        for e in &tags {
            assert_eq!(super::attributes::<Vec<_>>(e), checked(e), "{e:?}");
        }
        let a: Vec<(String, String)> = super::attributes(&tags[1]);
        assert_eq!(a.iter().filter(|(k, _)| k == "x").map(|(_, v)| v.as_str()).collect::<Vec<_>>(), ["1", "3"]);
        assert_eq!(super::attributes::<Vec<_>>(&tags[2]).len(), 40);
    }

    /// The malformed tags that read differently from quick-xml's check, as [`super::attributes`]
    /// says. The first copy of `r` has an unquoted value, a repeat of `x` has `y` written right
    /// after it, and a repeat of `x` has a space in its value. Each case fails if the check is
    /// turned back on.
    #[test]
    fn a_malformed_tag_keeps_the_first_copy_that_reads() {
        let tags = tags(r#"<r><c r=5 r="7"/><c x="1" x="2"y="3"/><c x="1" x="2 z='9'"/></r>"#);
        let pairs = |p: &[(&str, &str)]| p.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        let read = |i: usize| (super::attributes::<Vec<_>>(&tags[i]), checked(&tags[i]));
        assert_eq!(read(1), (pairs(&[("r", "7")]), pairs(&[])));
        assert_eq!(read(2), (pairs(&[("x", "1"), ("y", "3")]), pairs(&[("x", "1")])));
        assert_eq!(read(3), (pairs(&[("x", "1")]), pairs(&[("x", "1"), ("z", "9")])));
    }
}
