const drop = document.getElementById("drop");
const fileInput = document.getElementById("file");
const info = document.getElementById("info");
const btnPlay = document.getElementById("btnPlay");
const btnPause = document.getElementById("btnPause");
const audioEl = document.getElementById("audio");
const cv = document.getElementById("cv");
const ctx = cv.getContext("2d");

const PDFJS_CDN = "https://cdn.jsdelivr.net/npm/pdfjs-dist@4.10.38/build/pdf.min.js";
const PDFJS_WORKER_CDN = "https://cdn.jsdelivr.net/npm/pdfjs-dist@4.10.38/build/pdf.worker.min.js";

async function ensurePdfJs() {
  if (window.pdfjsLib) return;
  await new Promise((resolve, reject) => {
    const s = document.createElement("script");
    s.src = PDFJS_CDN;
    s.onload = resolve;
    s.onerror = () => reject(new Error("Failed to load PDF.js from CDN"));
    document.head.appendChild(s);
  });
  if (!window.pdfjsLib) throw new Error("pdfjsLib is not defined");
  if (pdfjsLib.GlobalWorkerOptions) {
    pdfjsLib.GlobalWorkerOptions.workerSrc = PDFJS_WORKER_CDN;
  }
}

let state = {
  loaded: false,
  fps: 30,
  w: 96,
  h: 72,
  frames: 0,
  headerSize: 10,
  packedLen: 0,
  blob: null,     // decoded BA bytes (header+frames)
  cur: null,      // Uint8Array current bitset
  off: 0,
  frameIndex: 0,
  startClock: 0,
  raf: 0,
  img: null,
  audioUrl: null,
};

// bit helper (MSB-first)
function getBit(buf, i) {
  const b = buf[i >> 3];
  const shift = 7 - (i & 7);
  return (b >> shift) & 1;
}

function xorInPlace(dst, src) {
  for (let i = 0; i < dst.length; i++) dst[i] ^= src[i];
}

function parseHeader(u8) {
  if (u8.byteLength < 10) throw new Error("BA stream too small for header");
  const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength);
  const w = dv.getUint16(0, true);
  const h = dv.getUint16(2, true);
  const fps_x100 = dv.getUint16(4, true);
  const frames = dv.getUint32(6, true);
  if (!w || !h || !frames) throw new Error("Invalid BA header values");
  return { w, h, fps: fps_x100 / 100.0, frames, headerSize: 10 };
}

function renderFrame(bitset, w, h) {
  if (!state.img || state.img.width !== w || state.img.height !== h) {
    state.img = ctx.createImageData(w, h);
  }
  const data = state.img.data;
  let p = 0;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = y * w + x;
      const v = getBit(bitset, i) ? 0 : 255;
      data[p++] = v; data[p++] = v; data[p++] = v; data[p++] = 255;
    }
  }
  ctx.putImageData(state.img, 0, 0);
}

async function extractXObjectBytes(pdf, name) {
  const page = await pdf.getPage(1);

  // 내부 접근(버전 의존). MVP 용.
  const transport = pdf._transport;
  const xref = transport.xref;

  const pageDict = page._pageDictionary || (page._pageInfo && page._pageInfo.pageDict);
  if (!pageDict) throw new Error("Cannot access page dictionary (PDF.js internals changed).");

  const res = await pageDict.get("Resources");
  const xobj = await res.get("XObject");
  const ref = await xobj.get(name); // "BA" or "AU"

  const stream = await xref.fetch(ref);

  // getBytes()는 보통 Filter가 적용된 “디코딩된 바이트”를 줌
  // (우리는 PDF에 FlateDecode로 넣을 예정)
  const bytes = stream.getBytes();
  return new Uint8Array(bytes);
}

function stopPlayback() {
  if (state.raf) cancelAnimationFrame(state.raf);
  state.raf = 0;
}

function startPlayback() {
  stopPlayback();
  state.frameIndex = 0;
  state.off = state.headerSize;

  // frame0
  state.cur = new Uint8Array(state.packedLen);
  state.cur.set(state.blob.subarray(state.off, state.off + state.packedLen));
  state.off += state.packedLen;

  // 오디오를 “마스터 클럭”으로 사용 (동기화 안정)
  state.startClock = performance.now();

  const tick = () => {
    // 오디오가 재생중이면 오디오 시간을 기준으로 프레임 맞추기
    let t = audioEl && !audioEl.paused ? audioEl.currentTime : ((performance.now() - state.startClock) / 1000);
    const target = Math.floor(t * state.fps);

    while (state.frameIndex < target && state.frameIndex + 1 < state.frames) {
      state.frameIndex++;
      const diff = state.blob.subarray(state.off, state.off + state.packedLen);
      state.off += state.packedLen;
      xorInPlace(state.cur, diff);
    }

    renderFrame(state.cur, state.w, state.h);

    if (state.frameIndex + 1 >= state.frames) return;
    state.raf = requestAnimationFrame(tick);
  };

  state.raf = requestAnimationFrame(tick);
}

async function loadPdfFile(file) {
  stopPlayback();
  audioEl.pause();
  audioEl.removeAttribute("src");
  audioEl.load();
  if (state.audioUrl) {
    URL.revokeObjectURL(state.audioUrl);
    state.audioUrl = null;
  }

  state.loaded = false;
  await ensurePdfJs();
  const ab = await file.arrayBuffer();
  const bytes = new Uint8Array(ab);

  const loadingTask = pdfjsLib.getDocument({ data: bytes });
  const pdf = await loadingTask.promise;

  // 1) BA 추출
  const ba = await extractXObjectBytes(pdf, "BA");
  const hdr = parseHeader(ba);

  state.w = hdr.w;
  state.h = hdr.h;
  state.fps = hdr.fps;
  state.frames = hdr.frames;
  state.headerSize = hdr.headerSize;
  state.packedLen = Math.ceil((state.w * state.h) / 8);
  state.blob = ba;
  const expected = state.headerSize + (state.packedLen * state.frames);
  if (state.blob.length < expected) {
    throw new Error(`BA stream truncated: expected ${expected} bytes, got ${state.blob.length}`);
  }

  // canvas 설정
  cv.width = state.w;
  cv.height = state.h;
  cv.style.width = (state.w * 6) + "px";
  cv.style.height = (state.h * 6) + "px";

  // 2) AU 추출 (오디오)
  const au = await extractXObjectBytes(pdf, "AU");

  // 오디오 포맷은 우리가 encoder에서 정할 것(추천: audio/ogg; codecs=opus)
  const audioBlob = new Blob([au], { type: "audio/ogg" });
  state.audioUrl = URL.createObjectURL(audioBlob);
  audioEl.src = state.audioUrl;

  state.loaded = true;
  btnPlay.disabled = false;
  btnPause.disabled = false;

  info.textContent = `Loaded from your PDF — ${state.w}x${state.h}, fps=${state.fps}, frames=${state.frames}`;

  // 자동 재생(사용자 제스처 직후라면 허용될 확률 높음)
  await audioEl.play().catch(() => {});
  startPlayback();
}

function handleFile(file) {
  if (!file) return;
  loadPdfFile(file).catch(e => {
    console.error(e);
    alert("Failed to load PDF: " + e.message);
  });
}

// drag & drop
drop.addEventListener("dragover", (e) => { e.preventDefault(); drop.style.borderColor = "#333"; });
drop.addEventListener("dragleave", () => { drop.style.borderColor = "#999"; });
drop.addEventListener("drop", (e) => {
  e.preventDefault();
  drop.style.borderColor = "#999";
  const f = e.dataTransfer.files && e.dataTransfer.files[0];
  handleFile(f);
});

fileInput.addEventListener("change", () => {
  const f = fileInput.files && fileInput.files[0];
  handleFile(f);
});

btnPlay.addEventListener("click", async () => {
  if (!state.loaded) return;
  await audioEl.play().catch(() => {});
  startPlayback();
});

btnPause.addEventListener("click", () => {
  if (!state.loaded) return;
  audioEl.pause();
  stopPlayback();
});
