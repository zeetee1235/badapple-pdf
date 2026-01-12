# badapple-pdf

https://github.com/kevinlinxc/badapple-pdf 이 프로젝트에 영감을 받음
Bad Apple 영상을 **PDF 안에 담아** 재생하는 데모 프로젝트.

이 프로젝트의 핵심은 “영상/오디오 데이터가 PDF 내부에 들어있다”는 점이다.  
PDF에는 `BA.bin`(프레임 데이터)과 `AU.ogg`(오디오)가 **첨부파일(EmbeddedFiles)** 형태로 포함되며, 브라우저에서 실행되는 플레이어가 사용자가 선택한 PDF에서 이 데이터를 추출해 캔버스와 오디오로 재생한다.

## 구성
- `encoder/` : Rust 인코더 (mp4/ogg → badapple.pdf 생성)
- `docs/` : GitHub Pages 웹 플레이어 (PDF.js 포함)
- `out/` : 테스트 출력물 (`badapple.pdf`)
- `run_test.sh` : 인코더 빌드 + PDF 생성 + 간단 검증

## PDF 내부 데이터 규약
PDF에는 다음 첨부파일이 반드시 포함된다(대소문자 포함).

- `BA.bin` : 영상 프레임 데이터 (raw)
- `AU.ogg` : 오디오 데이터 (raw, OGG/Opus 권장)

### `BA.bin` 포맷
- 헤더(LE)
  - `u16 width`
  - `u16 height`
  - `u16 fps_x100`
  - `u32 frame_count`
- `frame0` : raw bitset (MSB-first)
- `frame1..` : `prev XOR cur` diff bitset (동일 크기)

### `AU.ogg` 포맷
- OGG 바이트를 그대로 저장한다.

## 인코더 사용법
```bash
cargo run --release --manifest-path encoder/Cargo.toml -- \
  badapple.mp4 badapple.ogg out/badapple.pdf 80 60 30 128 0 \
  "https://zeetee1235.github.io/badapple-pdf/play.html"
