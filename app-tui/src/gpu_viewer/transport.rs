//! Versioned companion transport: small JSON metadata plus packed little-endian buffers.
//! Generic streams keep filesystem/process effects in the executable runner.
use super::scene::{Scene, ViewerProject, SAMPLE_BUDGET};
use serde::{Deserialize, Serialize};
use std::io::{BufReader, BufWriter, Read, Write};
const MAGIC: &[u8; 8] = b"FEAGPU01";
const HEADER_LIMIT: usize = 1024 * 1024;
#[derive(Serialize, Deserialize)]
struct Header {
    version: u32,
    selected: usize,
    problem: Option<fea_problem::Problem>,
    scenes: Vec<SceneHeader>,
}
#[derive(Serialize, Deserialize)]
struct SceneHeader {
    label: String,
    positions: usize,
    triangles: usize,
    samples: usize,
    times: Vec<f32>,
    range: [f32; 2],
    gain: f32,
    harmonic_period: Option<f32>,
}
pub fn write_project(output: impl Write, project: &ViewerProject) -> Result<(), String> {
    project.validate()?;
    let header = Header {
        version: project.version,
        selected: project.selected,
        problem: project.problem.clone(),
        scenes: project
            .scenes
            .iter()
            .map(|s| SceneHeader {
                label: s.label.clone(),
                positions: s.positions.len(),
                triangles: s.triangles.len(),
                samples: s.samples.len(),
                times: s.times.clone(),
                range: s.range,
                gain: s.gain,
                harmonic_period: s.harmonic_period,
            })
            .collect(),
    };
    let header = serde_json::to_vec(&header).map_err(|e| e.to_string())?;
    if header.len() > HEADER_LIMIT {
        return Err("native session metadata exceeds 1 MiB".into());
    }
    let mut output = BufWriter::new(output);
    (|| -> std::io::Result<()> {
        output.write_all(MAGIC)?;
        output.write_all(&(header.len() as u32).to_le_bytes())?;
        output.write_all(&header)?;
        for scene in &project.scenes {
            for v in scene.positions.iter().flatten() {
                output.write_all(&v.to_le_bytes())?;
            }
            for v in &scene.triangles {
                output.write_all(&v.to_le_bytes())?;
            }
            for v in scene.samples.iter().flatten() {
                output.write_all(&v.to_le_bytes())?;
            }
        }
        output.flush()
    })()
    .map_err(|e| e.to_string())
}
pub fn read_project(input: impl Read) -> Result<ViewerProject, String> {
    let mut input = BufReader::new(input);
    let mut magic = [0; 8];
    input.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if &magic != MAGIC {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Document {
            Project(ViewerProject),
            Legacy(Scene),
        }
        let project = match serde_json::from_reader(std::io::Cursor::new(magic).chain(input))
            .map_err(|e| e.to_string())?
        {
            Document::Project(p) => p,
            Document::Legacy(s) => ViewerProject::single(s),
        };
        project.validate()?;
        return Ok(project);
    }
    let read_u32 = |input: &mut BufReader<_>| -> Result<u32, String> {
        let mut bytes = [0; 4];
        input.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        Ok(u32::from_le_bytes(bytes))
    };
    let len = read_u32(&mut input)? as usize;
    if len > HEADER_LIMIT {
        return Err("oversized native session metadata".into());
    }
    let mut bytes = vec![0; len];
    input.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let header: Header = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if header.version != 1
        || header.scenes.is_empty()
        || header.scenes.len() > 64
        || header.selected >= header.scenes.len()
    {
        return Err("invalid native session header".into());
    }
    let mut samples = 0usize;
    let mut triangles = 0usize;
    for h in &header.scenes {
        samples = samples
            .checked_add(h.samples)
            .ok_or("sample count overflow")?;
        triangles = triangles
            .checked_add(h.triangles)
            .ok_or("triangle count overflow")?;
        if h.positions == 0
            || h.positions > 500_000
            || h.times.is_empty()
            || h.times.len() > 1001
            || h.positions.checked_mul(h.times.len()) != Some(h.samples)
            || samples > SAMPLE_BUDGET
            || h.triangles == 0
            || h.triangles > 3_000_000
            || h.triangles % 3 != 0
            || triangles > 12_000_000
        {
            return Err("oversized or inconsistent native session buffers".into());
        }
    }
    let mut scenes = Vec::new();
    for h in header.scenes {
        let mut positions = Vec::with_capacity(h.positions);
        for _ in 0..h.positions {
            positions.push([
                f32::from_bits(read_u32(&mut input)?),
                f32::from_bits(read_u32(&mut input)?),
                f32::from_bits(read_u32(&mut input)?),
            ]);
        }
        let mut triangles = Vec::with_capacity(h.triangles);
        for _ in 0..h.triangles {
            triangles.push(read_u32(&mut input)?);
        }
        let mut samples = Vec::with_capacity(h.samples);
        for _ in 0..h.samples {
            samples.push([
                f32::from_bits(read_u32(&mut input)?),
                f32::from_bits(read_u32(&mut input)?),
                f32::from_bits(read_u32(&mut input)?),
                f32::from_bits(read_u32(&mut input)?),
            ]);
        }
        scenes.push(Scene {
            label: h.label,
            positions,
            triangles,
            samples,
            times: h.times,
            range: h.range,
            gain: h.gain,
            harmonic_period: h.harmonic_period,
        });
    }
    let mut trailing = [0];
    if input.read(&mut trailing).map_err(|e| e.to_string())? != 0 {
        return Err("trailing native session data".into());
    }
    let project = ViewerProject {
        version: header.version,
        selected: header.selected,
        problem: header.problem,
        scenes,
    };
    project.validate()?;
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> ViewerProject {
        ViewerProject::single(Scene {
            label: "codec".into(),
            positions: vec![[0.0; 3]; 3],
            triangles: vec![0, 1, 2],
            samples: vec![[0.1, 0.2, 0.3, 1.0]; 3],
            times: vec![0.0],
            range: [0.0, 1.0],
            gain: 1.0,
            harmonic_period: Some(2.0),
        })
    }
    #[test]
    fn binary_and_legacy_json_round_trip_exactly() {
        let p = project();
        let mut bytes = Vec::new();
        write_project(&mut bytes, &p).unwrap();
        assert!(bytes.starts_with(MAGIC));
        let read = read_project(bytes.as_slice()).unwrap();
        assert_eq!(read.scenes[0].samples, p.scenes[0].samples);
        assert_eq!(read.scenes[0].harmonic_period, Some(2.0));
        assert_eq!(
            read_project(serde_json::to_vec(&p).unwrap().as_slice())
                .unwrap()
                .scenes[0]
                .samples,
            p.scenes[0].samples
        );
        assert_eq!(
            read_project(serde_json::to_vec(&p.scenes[0]).unwrap().as_slice())
                .unwrap()
                .scenes[0]
                .samples,
            p.scenes[0].samples
        );
        assert!(read_project(&bytes[..bytes.len() - 1]).is_err());
        bytes.push(1);
        assert!(read_project(bytes.as_slice()).is_err());
    }
    #[test]
    fn malformed_counts_are_refused_before_buffer_allocation() {
        let header = Header {
            version: 1,
            selected: 0,
            problem: None,
            scenes: vec![SceneHeader {
                label: "bad".into(),
                positions: usize::MAX,
                triangles: 3,
                samples: usize::MAX,
                times: vec![0.0],
                range: [0.0, 1.0],
                gain: 1.0,
                harmonic_period: None,
            }],
        };
        let h = serde_json::to_vec(&header).unwrap();
        let mut b = MAGIC.to_vec();
        b.extend((h.len() as u32).to_le_bytes());
        b.extend(h);
        assert!(read_project(b.as_slice()).unwrap_err().contains("buffers"));
        let mut b = MAGIC.to_vec();
        b.extend(((HEADER_LIMIT + 1) as u32).to_le_bytes());
        assert!(read_project(b.as_slice()).unwrap_err().contains("metadata"));
    }
}
