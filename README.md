# badapple-pdf

PDF 하나로 Bad Apple 영상을 재생하는 데모.  
겉으로는 PDF에 START 버튼이 보이고, 누르면 GitHub Pages의 웹 플레이어로 이동한다.  
속으로는 웹 플레이어가 사용자가 선택한 로컬 PDF에서 BA(영상) + AU(오디오)를 직접 추출해 재생한다.

## 구조 요약
- `encoder/`: Rust 인코더 (mp4/ogg -> badapple.pdf 생성)
- `docs/`: GitHub Pages 웹 플레이어 (PDF.js 로컬 번들 포함)
- `out/`: 테스트 출력물 (`badapple.pdf`)
- `run_test.sh`: 인코더 빌드 + PDF 생성 + 간단 검증

## PDF 내부 규약 (고정)
Page 1 -> Resources -> XObject
- `BA`: Bad Apple 영상 프레임 (FlateDecode로 압축)
- `AU`: 오디오 OGG (FlateDecode로 압축, `Mime=audio/ogg`)

### BA 포맷
- 헤더(LE): `u16 width`, `u16 height`, `u16 fps_x100`, `u32 frame_count`
- frame0: raw bitset (MSB-first)
- frame1..: 이전 프레임과 XOR한 diff bitset
- 전체 blob을 zlib(FlateDecode) 압축

### AU 포맷
- OGG(Opus 권장) 바이트 전체를 zlib(FlateDecode) 압축

## 인코더 사용법
예시:
```bash
cargo run --release --manifest-path encoder/Cargo.toml -- \
  badapple.mp4 badapple.ogg out/badapple.pdf 80 60 30 128 0 \
  "https://zeetee1235.github.io/badapple-pdf/play.html"
```

인자 설명:
- `<video.mp4> <audio.ogg> <out.pdf> <w> <h> <fps> <threshold> <max_frames_or_0> <start_url>`

## 웹 플레이어
- 파일: `docs/play.html`, `docs/app.js`
- PDF.js 로컬 번들: `docs/vendor/pdfjs/pdf.min.mjs`, `docs/vendor/pdfjs/pdf.worker.min.mjs`
- 동작: 사용자가 PDF를 드롭/선택 -> `BA`/`AU` 스트림 추출 -> 캔버스 + 오디오 재생

## 테스트
```bash
./run_test.sh
```
- `out/badapple.pdf` 생성
- `pdfinfo`, `qpdf --check`, `qpdf --show-npages`로 간단 검증

## GitHub Pages 배포
`docs/` 폴더를 Pages 소스로 사용.  
URL: `https://zeetee1235.github.io/badapple-pdf/play.html`
