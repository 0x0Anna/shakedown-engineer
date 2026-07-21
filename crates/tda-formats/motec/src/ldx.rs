//! Parser for the `.ldx` sidecar XML file some exporters write alongside
//! a `.ld` log.
//!
//! Observed from an Assetto Corsa Competizione export: unlike real MoTeC
//! hardware/software (which encodes lap-crossing events as trigger values
//! in the `.ld`'s own `Beacon` channel — see `tda-core`'s
//! `laps_from_beacon`), ACC's `.ld` export leaves that channel's samples
//! at a constant value and instead records the actual lap-crossing
//! timestamps in this separate `.ldx` XML file, as
//! `<Marker ... Time="1.234e+05"/>` elements (time in microseconds since
//! the start of the session) inside a `<MarkerGroup Name="Beacons">`.
//!
//! This module only extracts what's needed to derive lap boundaries
//! (marker times) plus a couple of informational summary fields; it does
//! not attempt to model the full `.ldx` schema (layers, ranges, locale,
//! etc.), none of which is relevant to this project.

use std::fs;
use std::path::Path;

use crate::error::LdxError;

/// Parsed contents of a `.ldx` sidecar file, reduced to what this project
/// needs: lap-crossing marker times, plus a few summary fields carried
/// straight from the file for display purposes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LdxFile {
    /// Lap-crossing timestamps, in milliseconds since the start of the
    /// session, sorted ascending. Converted from the file's native
    /// microsecond `Time` attribute.
    pub marker_times_ms: Vec<f64>,
    /// `<String Id="Total Laps" Value="...">`, if present.
    pub total_laps: Option<u32>,
    /// `<String Id="Fastest Lap" Value="...">`, if present (1-based lap
    /// number, per ACC's own numbering — not necessarily the same
    /// indexing as the `Lap::num` this project derives from markers).
    pub fastest_lap: Option<u32>,
    /// `<String Id="Fastest Time" Value="...">`, if present, e.g.
    /// `"2:00.717"`. Kept as a display string, not parsed into a
    /// duration — nothing downstream needs it as a number yet.
    pub fastest_time: Option<String>,
}

/// Parse a `.ldx` sidecar file from disk.
///
/// # Errors
///
/// Returns [`LdxError::Io`] if the file can't be read, or
/// [`LdxError::Xml`]/[`LdxError::BadMarkerTime`] if it can't be parsed —
/// see [`parse_ldx_str`].
pub fn parse_ldx(path: &Path) -> Result<LdxFile, LdxError> {
    let text = fs::read_to_string(path).map_err(|source| LdxError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_ldx_str(&text)
}

/// Parse already-loaded `.ldx` XML text.
///
/// # Errors
///
/// Returns [`LdxError::Xml`] if the text isn't well-formed XML, or
/// [`LdxError::BadMarkerTime`] if a `<Marker>` element's `Time` attribute
/// isn't a parseable number.
pub fn parse_ldx_str(text: &str) -> Result<LdxFile, LdxError> {
    let doc = roxmltree::Document::parse(text)?;

    let mut marker_times_ms = Vec::new();
    let mut total_laps = None;
    let mut fastest_lap = None;
    let mut fastest_time = None;

    for (index, node) in doc
        .descendants()
        .filter(|n| n.has_tag_name("Marker"))
        .enumerate()
    {
        let Some(time) = node.attribute("Time") else {
            continue;
        };
        let micros: f64 = time.parse().map_err(|_| LdxError::BadMarkerTime {
            index,
            value: time.to_string(),
        })?;
        marker_times_ms.push(micros / 1000.0);
    }
    marker_times_ms.sort_by(f64::total_cmp);

    for node in doc.descendants().filter(|n| n.has_tag_name("String")) {
        let (Some(id), Some(value)) = (node.attribute("Id"), node.attribute("Value")) else {
            continue;
        };
        match id {
            "Total Laps" => total_laps = value.parse().ok(),
            "Fastest Lap" => fastest_lap = value.parse().ok(),
            "Fastest Time" => fastest_time = Some(value.to_string()),
            _ => {}
        }
    }

    Ok(LdxFile {
        marker_times_ms,
        total_laps,
        fastest_lap,
        fastest_time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<LDXFile Version="1.6">
 <Layers>
  <Layer>
   <MarkerBlock>
    <MarkerGroup Name="Beacons" Index="0">
     <Marker Version="100" ClassName="BCN" Name="1, id=99" Flags="13" Time="7.920000000000000e+05"/>
     <Marker Version="100" ClassName="BCN" Name="2, id=99" Flags="13" Time="1.215480000000000e+08"/>
     <Marker Version="100" ClassName="BCN" Name="3, id=99" Flags="13" Time="2.422650000000000e+08"/>
    </MarkerGroup>
   </MarkerBlock>
   <RangeBlock/>
  </Layer>
 </Layers>
 <Details>
  <String Id="Total Laps" Value="6"/>
  <String Id="Fastest Time" Value="2:00.717"/>
  <String Id="Fastest Lap" Value="3"/>
 </Details>
</LDXFile>
"#;

    #[test]
    fn parses_marker_times_and_summary_fields() {
        let ldx = parse_ldx_str(SAMPLE).unwrap();
        assert_eq!(ldx.marker_times_ms, vec![792.0, 121_548.0, 242_265.0]);
        assert_eq!(ldx.total_laps, Some(6));
        assert_eq!(ldx.fastest_lap, Some(3));
        assert_eq!(ldx.fastest_time.as_deref(), Some("2:00.717"));
    }

    #[test]
    fn empty_document_has_no_markers_and_no_summary() {
        let ldx = parse_ldx_str("<LDXFile Version=\"1.6\"></LDXFile>").unwrap();
        assert!(ldx.marker_times_ms.is_empty());
        assert_eq!(ldx.total_laps, None);
    }

    #[test]
    fn malformed_xml_is_rejected() {
        assert!(parse_ldx_str("<LDXFile>").is_err());
    }

    #[test]
    fn unparseable_marker_time_is_rejected() {
        let xml = r#"<LDXFile><Marker Time="not-a-number"/></LDXFile>"#;
        let err = parse_ldx_str(xml).unwrap_err();
        assert!(matches!(err, LdxError::BadMarkerTime { index: 0, .. }));
    }
}
