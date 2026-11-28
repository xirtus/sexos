#!/usr/bin/env bash
# SexTime civil/Gregorian/Long Count conversions: crate unit tests, a bare
# no_std target build and an independent Python calendar oracle.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-sex-time.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
SRC="$ROOT_DIR/crates/sex-time/src/lib.rs"
source "$(dirname "${BASH_SOURCE[0]}")/dreamroot_core_rlib.sh"
dreamroot_core_rlib "$TEST_DIR/host"
dreamroot_core_rlib "$TEST_DIR/none" --target x86_64-unknown-none -O
DREAMROOT=(--extern dreamroot_core="$TEST_DIR/host/libdreamroot_core.rlib")
rustc --edition=2021 "${DREAMROOT[@]}" --test "$SRC" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
rustc --edition=2021 --crate-type rlib --crate-name sex_time --target x86_64-unknown-none -O --extern dreamroot_core="$TEST_DIR/none/libdreamroot_core.rlib" "$SRC" -o "$TEST_DIR/libsex_time.rlib"
echo '[sex_time.no_std] PASS target=x86_64-unknown-none'
rustc --edition=2021 --crate-type rlib --crate-name sex_time "${DREAMROOT[@]}" "$SRC" -o "$TEST_DIR/libsex_time_host.rlib"
cat > "$TEST_DIR/probe.rs" <<'RS'
use std::io::BufRead;
use sex_time::CivilDay;
fn main() {
    for line in std::io::stdin().lock().lines() {
        let unix: i64 = line.unwrap().trim().parse().unwrap();
        let ts = sex_time::SexTimestamp::from_unix(unix, 0).unwrap();
        let g = sex_time::Gregorian::from_timestamp(ts).unwrap();
        let lc = sex_time::LongCount::from_timestamp(ts).unwrap();
        println!("{} {} {:04}-{:02}-{:02}T{:02}:{:02}:{:02} {} {}.{}.{}.{}.{}", unix, ts.seconds(), g.year, g.month, g.day,
            g.hour, g.minute, g.second, ts.julian_day_number(), lc.baktun, lc.katun, lc.tun, lc.winal, lc.kin);
    }
}
RS
rustc --edition=2021 --extern sex_time="$TEST_DIR/libsex_time_host.rlib" -L dependency="$TEST_DIR/host" "$TEST_DIR/probe.rs" -o "$TEST_DIR/probe"
python3 - "$TEST_DIR/probe" <<'PY'
import datetime,random,subprocess,sys
# Independent oracle: Python's proleptic Gregorian calendar, its ordinal day
# (JDN = ordinal + 1721424.5 at midnight, so 1721425 for the civil day) and
# Long Count place values derived from GMT 584283.
rng=random.Random(20121221)
epoch=datetime.datetime(1970,1,1,tzinfo=datetime.timezone.utc)
lo=int((datetime.datetime(1,1,1,tzinfo=datetime.timezone.utc)-epoch).total_seconds())
hi=int((datetime.datetime(9999,12,31,23,59,59,tzinfo=datetime.timezone.utc)-epoch).total_seconds())
samples=[lo,hi,0,1356048000,1356047999,1356134400,951782400,4107456000]+[rng.randrange(lo,hi) for _ in range(5000)]
out=subprocess.run([sys.argv[1]],input='\n'.join(map(str,samples))+'\n',capture_output=True,text=True,check=True).stdout.splitlines()
assert len(out)==len(samples)
for unix,line in zip(samples,out):
    when=epoch+datetime.timedelta(seconds=unix);jdn=when.date().toordinal()+1721425;days=jdn-584283
    places=[days//144000,days%144000//7200,days%7200//360,days%360//20,days%20]
    want=f"{unix} {unix-1356048000} {when:%Y-%m-%dT%H:%M:%S} {jdn} {'.'.join(map(str,places))}"
    assert line==want,(line,want)
print(f'[sex_time.oracle] PASS samples={len(samples)} years=1..9999')
PY
echo '[sex_time.host.result] PASS scope=civil_calendar_conversion_only_no_clock_source_trust_or_abi_change'
