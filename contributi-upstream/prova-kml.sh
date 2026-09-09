#!/usr/bin/env bash
# La proposta per georust/kml: modifica minima, riproduttori, regressioni.
# Prepara e verifica su `main`. NON spinge: l'invio e' una decisione separata.
set -euo pipefail
export CARGO_TARGET_DIR=/tmp/kml-prop-target

D=/tmp/kml-proposta
rm -rf "$D"
git clone -q https://github.com/georust/kml "$D"
cd "$D"
echo "base: main @ $(git rev-parse --short HEAD)"

python3 - src/reader.rs <<'PY'
import io, sys

p = sys.argv[1]
s = io.open(p, encoding="utf-8", newline="").read()

# I quattro cicli **misurati** come non terminanti. `read_geom_props` ne copre
# tre elementi (Point, LineString, LinearRing), gli altri uno ciascuno.
#
# Ogni ciclo aspetta un `Event::End` preciso e ignora tutto il resto; a fine
# documento `next_event` restituisce `Eof` all'infinito. `read_elements` non e'
# toccata: li' `Eof` significa davvero «finito», ed e' l'unico posto in cui lo
# significa. Nemmeno `next_event` e' toccata: `read_elements` e' rientrante e un
# documento legittimo consuma piu' di un `Eof`, quindi un contatore globale
# rifiuterebbe file validi -- misurato, quattro prove del crate diventavano rosse.
ARM = """                Event::Eof => {
                    return Err(Error::InvalidInput(
                        "document ended before %s was closed".to_string(),
                    ))
                }
"""

CASI = [
    # (ancora unica da cui parte il ciclo, cio' che il messaggio nomina)
    ("""    fn read_geom_props(&mut self, end_tag: &[u8]) -> Result<GeomProps<T>, Error> {""",
     "the geometry"),
    ("""    fn read_placemark(&mut self, attrs: HashMap<String, String>) -> Result<Placemark<T>, Error> {""",
     "the Placemark"),
    ("""    fn read_data(&mut self, mut attrs: HashMap<String, String>) -> Result<Data, Error> {""",
     "the Data element"),
    ("""    fn read_schema_data(&mut self, attrs: HashMap<String, String>) -> Result<SchemaData, Error> {""",
     "the SchemaData element"),
]

for firma, nome in CASI:
    assert s.count(firma) == 1, (s.count(firma), firma[:60])
    inizio = s.index(firma)
    # Il primo `_ => {}` dopo la firma e' il ramo di scarto del ciclo: il ramo
    # `Eof` va **prima**, altrimenti lo scarto lo mangerebbe.
    scarto = "\n                _ => {}\n"
    i = s.index(scarto, inizio)
    s = s[:i + 1] + (ARM % nome) + s[i + 1:]

io.open(p, "w", encoding="utf-8", newline="").write(s)
print("quattro rami Eof innestati")
PY

python3 - src/reader.rs <<'PY'
import io, sys

p = sys.argv[1]
s = io.open(p, encoding="utf-8", newline="").read()
ancora = "mod tests {\n"
assert s.count(ancora) == 1
prove = '''mod tests {
    /// A truncated element must not keep the reader spinning.
    ///
    /// These six hang on `main`: the parse never returns. So this test would
    /// not go red if the defect came back - it would hang, and CI would time
    /// out instead. That is a property of the defect, and the reason the list
    /// is asserted rather than described in a comment.
    ///
    /// Only these six were measured to hang. Other loops in this file wait for
    /// an end tag without an `Eof` arm too and still return, so the missing arm
    /// is not by itself the defect, and they are left alone.
    #[test]
    fn truncated_elements_terminate() {
        for element in [
            "Point",
            "LineString",
            "LinearRing",
            "Placemark",
            "Data",
            "SchemaData",
        ] {
            let doc = format!("<{element}>");
            assert!(
                doc.parse::<Kml>().is_err(),
                "<{element}> without its end tag should be rejected, not spun on"
            );
        }
    }

    /// The shape a fuzzer produced: two `Point` start tags and no end tag.
    #[test]
    fn repeated_unclosed_point_terminates() {
        assert!("<xjA:Point>0j><xjA:Point>".parse::<Kml>().is_err());
    }

    /// The counter-proof: well formed documents still parse.
    ///
    /// Without it, a change that rejected everything would pass the two tests
    /// above. The rest of the suite covers this far better; this is here so the
    /// three tests read as one argument.
    #[test]
    fn well_formed_documents_still_parse() {
        let doc = "<Placemark><name>x</name>\\
                   <Point><coordinates>1,2</coordinates></Point></Placemark>";
        assert!(doc.parse::<Kml>().is_ok());
    }

'''
io.open(p, "w", encoding="utf-8", newline="").write(s.replace(ancora, prove, 1))
print("tre prove innestate")
PY

cargo fmt
echo "== le prove nuove"
cargo test truncated_elements_terminate 2>&1 | grep -E "^test |^test result|^error|-->" | head -6
cargo test repeated_unclosed_point 2>&1 | grep -E "^test |^test result|^error" | head -3
cargo test well_formed_documents 2>&1 | grep -E "^test |^test result|^error" | head -3
echo "== la suite del crate"
cargo test 2>&1 | grep -E "^test result|^failures:|^    [a-z_:]+$|^error" | head -12
cargo test >/dev/null 2>&1 || { echo "SUITE ROSSA: la proposta non e' pronta"; exit 1; }
echo "== clippy con -D warnings, la severita' della loro CI"
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings >/dev/null 2>&1 || { echo "CLIPPY ROSSO"; exit 1; }
echo "   pulito"
cargo fmt -- --check && echo "fmt pulito"

echo
echo "== il diff proposto"
git --no-pager diff --stat
mkdir -p /work/contributi-upstream/kml-cicli-senza-eof
git --no-pager diff > /work/contributi-upstream/kml-cicli-senza-eof/patch.diff
echo "   $(wc -l < /work/contributi-upstream/kml-cicli-senza-eof/patch.diff) righe di patch"
