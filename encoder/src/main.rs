use anyhow::{bail, Context, Result};
use lopdf::{dictionary, Dictionary, Document, Object, Stream};
use std::{
    env,
    fs,
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
};

// MSB-first bit packing (player.js getBit()와 동일 규약)
fn pack_bits(bits01: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; (bits01.len() + 7) / 8];
    for (i, &b) in bits01.iter().enumerate() {
        if b != 0 {
            out[i / 8] |= 1 << (7 - (i % 8));
        }
    }
    out
}

fn xor_bytes_inplace(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.iter_mut().zip(src.iter()) {
        *d ^= *s;
    }
}

/// ffmpeg로 raw gray 프레임을 stdout 파이프로 받는다.
/// - fps, scale, format=gray 고정
fn encode_video_blob_via_ffmpeg(
    video_path: &PathBuf,
    w: u16,
    h: u16,
    fps: f32,
    threshold: u8,
    max_frames: Option<u32>,
) -> Result<Vec<u8>> {
    let fps_str = if fps > 0.0 { fps.to_string() } else { "30".to_string() };

    // ffmpeg filter: fps=...,scale=WxH,format=gray
    let vf = format!("fps={},scale={}:{},format=gray", fps_str, w, h);

    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            video_path.to_string_lossy().as_ref(),
            "-vf",
            &vf,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn ffmpeg (is it installed?)")?;

    let mut stdout = child.stdout.take().context("failed to take ffmpeg stdout")?;

    let frame_sz = (w as usize) * (h as usize);
    let mut frame_buf = vec![0u8; frame_sz];

    // header (나중에 frame_count patch)
    // u16 w, u16 h, u16 fps_x100, u32 frame_count
    let mut blob: Vec<u8> = Vec::new();
    blob.extend_from_slice(&w.to_le_bytes());
    blob.extend_from_slice(&h.to_le_bytes());
    let fps_x100: u16 = (fps * 100.0).round().clamp(1.0, 65535.0) as u16;
    blob.extend_from_slice(&fps_x100.to_le_bytes());
    blob.extend_from_slice(&0u32.to_le_bytes()); // frame_count placeholder

    let packed_len = (frame_sz + 7) / 8;
    let mut prev_packed = vec![0u8; packed_len];
    let mut frame_count: u32 = 0;

    loop {
        if let Some(m) = max_frames {
            if frame_count >= m {
                break;
            }
        }

        // raw gray 한 프레임 읽기
        let mut read_total = 0usize;
        while read_total < frame_sz {
            let n = stdout.read(&mut frame_buf[read_total..])?;
            if n == 0 {
                // EOF
                read_total = 0;
                break;
            }
            read_total += n;
        }
        if read_total == 0 {
            break;
        }

        // threshold → bits01 (1=black, 0=white)
        let mut bits01 = vec![0u8; frame_sz];
        for (i, &px) in frame_buf.iter().enumerate() {
            bits01[i] = if px <= threshold { 1 } else { 0 };
        }

        let packed = pack_bits(&bits01);

        if frame_count == 0 {
            blob.extend_from_slice(&packed);
            prev_packed.copy_from_slice(&packed);
        } else {
            let mut diff = prev_packed.clone();
            xor_bytes_inplace(&mut diff, &packed); // diff = prev XOR cur
            blob.extend_from_slice(&diff);
            prev_packed.copy_from_slice(&packed);
        }

        frame_count += 1;
    }

    let status = child.wait()?;
    if !status.success() {
        bail!("ffmpeg exited with non-zero status");
    }

    // frame_count patch
    let fc_bytes = frame_count.to_le_bytes();
    blob[6..10].copy_from_slice(&fc_bytes);

    Ok(blob)
}

/// PDF 생성:
/// - 1페이지 컨텐츠에 START 버튼처럼 보이게 그려놓고
/// - 같은 영역에 Link annotation (/URI)을 올린다.
/// - EmbeddedFiles에 BA.bin / AU.ogg를 첨부한다.
fn add_attachment(doc: &mut Document, name: &str, data: &[u8], mime: &str) -> lopdf::ObjectId {
    let ef_id = doc.new_object_id();
    let ef_stream = Stream::new(
        dictionary! {
            "Type" => "EmbeddedFile",
            "Subtype" => mime,
            "Length" => data.len() as i64,
        },
        data.to_vec(),
    );
    doc.objects.insert(ef_id, Object::Stream(ef_stream));

    let filespec_id = doc.new_object_id();
    let filespec = dictionary! {
        "Type" => "Filespec",
        "F" => Object::String(name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
        "UF" => Object::String(name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
        "EF" => dictionary! {
            "F" => Object::Reference(ef_id),
        },
    };
    doc.objects.insert(filespec_id, Object::Dictionary(filespec));
    filespec_id
}

fn make_pdf(out_pdf: &PathBuf, start_url: &str, ba_raw: &[u8], au_raw: &[u8]) -> Result<()> {
    let mut doc = Document::with_version("1.7");

    // Object IDs
    let catalog_id = doc.new_object_id();
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();

    // Font object (Helvetica)
    let font_id = doc.new_object_id();
    doc.objects.insert(
        font_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica"
        }),
    );

    // Attachments (EmbeddedFiles)
    let ba_filespec_id = add_attachment(&mut doc, "BA.bin", ba_raw, "application/octet-stream");
    let au_filespec_id = add_attachment(&mut doc, "AU.ogg", au_raw, "audio/ogg");

    let names_id = doc.new_object_id();
    let embedded_files = dictionary! {
        "Names" => vec![
            Object::String("AU.ogg".as_bytes().to_vec(), lopdf::StringFormat::Literal),
            Object::Reference(au_filespec_id),
            Object::String("BA.bin".as_bytes().to_vec(), lopdf::StringFormat::Literal),
            Object::Reference(ba_filespec_id),
        ]
    };
    doc.objects.insert(
        names_id,
        Object::Dictionary(dictionary! { "EmbeddedFiles" => embedded_files }),
    );

    // Page Resources: Font only
    let resources = dictionary! {
        "Font" => dictionary! {
            "F1" => Object::Reference(font_id),
        }
    };

    // Page content: START 버튼처럼 보이도록 사각형+텍스트 그리기
    // 좌표: PDF point (612x792)
    // 버튼 영역 Rect = [x1 y1 x2 y2]
    let x1 = 156.0;
    let y1 = 360.0;
    let x2 = 456.0;
    let y2 = 460.0;

    let content = format!(
        "q\n\
         0.9 g\n\
         {x1} {y1} {w} {h} re\n\
         f\n\
         0 g\n\
         2 w\n\
         {x1} {y1} {w} {h} re\n\
         S\n\
         BT\n\
         /F1 36 Tf\n\
         {tx} {ty} Td\n\
         (START) Tj\n\
         ET\n\
         Q\n",
        x1 = x1,
        y1 = y1,
        w = x2 - x1,
        h = y2 - y1,
        tx = x1 + 80.0,
        ty = y1 + 35.0
    );

    let contents_id = doc.new_object_id();
    doc.objects.insert(
        contents_id,
        Object::Stream(Stream::new(dictionary! { "Length" => content.as_bytes().len() as i64 }, content.into_bytes())),
    );

    // Link annotation overlay
    let annot_id = doc.new_object_id();
    let annot = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => vec![
            Object::Real(x1),
            Object::Real(y1),
            Object::Real(x2),
            Object::Real(y2),
        ],
        "Border" => vec![0.into(), 0.into(), 0.into()],
        "A" => dictionary! {
            "S" => "URI",
            "URI" => Object::String(start_url.as_bytes().to_vec(), lopdf::StringFormat::Literal),
        }
    };
    doc.objects.insert(annot_id, Object::Dictionary(annot));

    // Page dictionary
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(pages_id),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources,
            "Contents" => Object::Reference(contents_id),
            "Annots" => vec![Object::Reference(annot_id)]
        }),
    );

    // Pages + Catalog
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1
        }),
    );
    doc.objects.insert(
        catalog_id,
        Object::Dictionary(dictionary! {
            "Type" => "Catalog",
            "Pages" => Object::Reference(pages_id),
            "Names" => Object::Reference(names_id),
            "AF" => vec![Object::Reference(ba_filespec_id), Object::Reference(au_filespec_id)],
        }),
    );
    doc.trailer.set("Root", Object::Reference(catalog_id));

    // 저장
    doc.save(out_pdf).context("failed to save pdf")?;
    Ok(())
}

fn parse_args() -> Result<(PathBuf, PathBuf, PathBuf, u16, u16, f32, u8, Option<u32>, String)> {
    // 사용법:
    // cargo run --release -- video.mp4 audio.ogg out.pdf 160 120 30 128 0 https://.../play.html
    let a: Vec<String> = env::args().collect();
    if a.len() < 10 {
        eprintln!("Usage:");
        eprintln!("  {} <video.mp4> <audio.ogg> <out.pdf> <w> <h> <fps> <threshold> <max_frames_or_0> <start_url>", a[0]);
        bail!("not enough args");
    }
    let video = PathBuf::from(&a[1]);
    let audio = PathBuf::from(&a[2]);
    let out = PathBuf::from(&a[3]);
    let w: u16 = a[4].parse()?;
    let h: u16 = a[5].parse()?;
    let fps: f32 = a[6].parse()?;
    let threshold: u8 = a[7].parse()?;
    let mf: u32 = a[8].parse()?;
    let max_frames = if mf == 0 { None } else { Some(mf) };
    let start_url = a[9].clone();
    Ok((video, audio, out, w, h, fps, threshold, max_frames, start_url))
}

fn main() -> Result<()> {
    let (video, audio, out_pdf, w, h, fps, threshold, max_frames, start_url) = parse_args()?;

    // 1) BA blob 생성 (raw, uncompressed)
    let ba_blob = encode_video_blob_via_ffmpeg(&video, w, h, fps, threshold, max_frames)
        .context("failed to encode video frames")?;
    eprintln!("BA blob (raw) bytes: {}", ba_blob.len());

    // 2) AU bytes 읽기 (raw)
    let au_raw = fs::read(&audio).context("failed to read audio file")?;
    eprintln!("AU raw bytes: {}", au_raw.len());

    // 3) PDF 생성 (attachments)
    if let Some(parent) = out_pdf.parent() {
        fs::create_dir_all(parent).ok();
    }
    make_pdf(&out_pdf, &start_url, &ba_blob, &au_raw)?;
    eprintln!("Wrote PDF: {}", out_pdf.display());

    Ok(())
}
