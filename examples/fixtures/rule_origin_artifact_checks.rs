use fluxr::{RuleProgram, hir::Hir, structured::Module};

const SIGNED: &[u8] = include_bytes!("../../tests/fixtures/composed-rule/signed.fxh");
const MIXED: &[u8] = include_bytes!("../../tests/fixtures/composed-rule/mixed.fxh");
const REFUSALS: &str = include_str!("../../tests/fixtures/composed-rule/refusals.tsv");

fn mutate(bytes: &mut Vec<u8>, recipe: &str) {
    let fields: Vec<_> = recipe.split_ascii_whitespace().collect();
    let number = |field: &str| field.parse::<usize>().unwrap();
    match fields.as_slice() {
        ["cut", length] => bytes.truncate(number(length)),
        ["append", byte] => bytes.push(u8::try_from(number(byte)).unwrap()),
        ["write", offset, values @ ..] => {
            let offset = number(offset);
            for (slot, byte) in bytes[offset..offset + values.len()].iter_mut().zip(values) {
                *slot = u8::try_from(number(byte)).unwrap();
            }
        }
        ["fill", length] => bytes.resize(number(length), b' '),
        _ => panic!("unknown fixture recipe {recipe}"),
    }
}

pub fn check() -> usize {
    let mut checked = 0;
    for row in REFUSALS.lines().skip(1) {
        let fields: Vec<_> = row.split('\t').collect();
        let [case, control, section, recipe, expected] = fields.as_slice() else {
            panic!("invalid refusal row {row}");
        };
        let seed = match *control {
            "signed" => SIGNED,
            "mixed" => MIXED,
            _ => panic!("unknown control {control}"),
        };
        let mut payload = seed[10..].to_vec();
        if *section == "payload" {
            mutate(&mut payload, recipe);
        } else {
            let origin_size = usize::from(u16::from_le_bytes([payload[0], payload[1]]));
            let mut origin = payload[2..2 + origin_size].to_vec();
            let mut projection = payload[2 + origin_size..].to_vec();
            match *section {
                "origin" => {
                    mutate(&mut origin, recipe);
                    assert_eq!(
                        RuleProgram::decode(&origin).unwrap_err().to_string(),
                        *expected,
                        "{case} FXD"
                    );
                }
                "projection" => {
                    mutate(&mut projection, recipe);
                    if *case == "changed-signed-projection" {
                        let source = std::str::from_utf8(&projection).unwrap();
                        let module = Module::compile_compact(source).unwrap();
                        assert_eq!(module.compact_text().unwrap(), source);
                    }
                }
                _ => panic!("unknown section {section}"),
            }
            payload = u16::try_from(origin.len()).unwrap().to_le_bytes().to_vec();
            payload.extend(origin);
            payload.extend(projection);
        }
        let mut artifact = seed[..6].to_vec();
        artifact.extend(u32::try_from(payload.len()).unwrap().to_le_bytes());
        artifact.extend(payload);
        let Err(error) = Hir::decode(&artifact) else {
            panic!("{case} accepted");
        };
        assert_eq!(error.to_string(), *expected, "{case} FXH");
        checked += 1;
    }
    checked
}
