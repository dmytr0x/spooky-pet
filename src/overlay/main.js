import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { splitToViewport } from "./chunks.js";
import { chunkVisibleMs } from "./delay.js";

const STATES = ["idle", "lurking", "talking", "muted"];
const CLICK_SLOP = 4;
const FONT_MIN_PX = 11;
const FONT_MAX_PX = 32;
const FONT_DEFAULT_PX = 18;
const OPACITY_MIN = 0.2;
const OPACITY_MAX = 1;
const OPACITY_DEFAULT = 0.9;
const DELAY_MIN_MS = 1000;
const DELAY_MAX_MS = 60000;
const DELAY_DEFAULT_MS = 8000;

const root = document.getElementById("root");
const pet = document.getElementById("pet");
const speech = document.getElementById("speech");
const speechText = document.getElementById("speech-text");
const overlay = getCurrentWindow();
const sfx = {
  lurk: document.getElementById("sfx-lurk"),
  speak: document.getElementById("sfx-speak"),
};

let dragging = false;
let press = null;
let fontMaxPx = FONT_DEFAULT_PX;
let jokeText = "";
let chunkDelayMs = DELAY_DEFAULT_MS;
let presentGen = 0;
let chunkTimer = null;

function clampFontSize(value) {
  const n = Math.round(Number(value));
  if (!Number.isFinite(n)) {
    return FONT_DEFAULT_PX;
  }
  return Math.min(FONT_MAX_PX, Math.max(FONT_MIN_PX, n));
}

function clampOpacity(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) {
    return OPACITY_DEFAULT;
  }
  return Math.min(OPACITY_MAX, Math.max(OPACITY_MIN, n));
}

function clampDelayMs(value) {
  const n = Math.round(Number(value));
  if (!Number.isFinite(n)) {
    return DELAY_DEFAULT_MS;
  }
  return Math.min(DELAY_MAX_MS, Math.max(DELAY_MIN_MS, n));
}

function applyPopupOpacity(value) {
  speech.style.opacity = String(clampOpacity(value));
}

function fitsViewport(el) {
  return el.scrollHeight <= el.clientHeight && el.scrollWidth <= el.clientWidth;
}

function fitsText(text) {
  speechText.textContent = text;
  return fitsViewport(speechText);
}

function splitIntoChunks(text) {
  return splitToViewport(text, fitsText);
}

function shrinkToFit() {
  let size = fontMaxPx;
  speechText.style.fontSize = `${size}px`;
  while (size > FONT_MIN_PX && !fitsViewport(speechText)) {
    size -= 1;
    speechText.style.fontSize = `${size}px`;
  }
}

function stopChunks() {
  presentGen += 1;
  if (chunkTimer !== null) {
    clearTimeout(chunkTimer);
    chunkTimer = null;
  }
}

function startPresentation() {
  stopChunks();
  const gen = presentGen;
  if (!root.classList.contains("talking")) {
    return;
  }
  speechText.style.fontSize = `${fontMaxPx}px`;
  if (!jokeText) {
    if (!speechText.textContent) {
      speechText.textContent = "…";
    }
    return;
  }
  const chunks = splitIntoChunks(jokeText);
  let index = 0;
  const show = () => {
    if (gen !== presentGen) {
      return;
    }
    speechText.style.fontSize = `${fontMaxPx}px`;
    speechText.textContent = chunks[index] ?? "";
    shrinkToFit();
    const visibleMs = chunkVisibleMs(chunks[index], chunkDelayMs);
    index += 1;
    if (index < chunks.length) {
      chunkTimer = setTimeout(show, visibleMs);
      return;
    }
    chunkTimer = setTimeout(() => {
      if (gen !== presentGen) {
        return;
      }
      invoke("dismiss_joke").catch((err) => {
        console.error(err);
      });
    }, visibleMs);
  };
  show();
}

function setPetState(state) {
  if (!STATES.includes(state)) {
    return;
  }
  const wasTalking = root.classList.contains("talking");
  root.className = state;
  if (state === "talking" && !wasTalking) {
    startPresentation();
  }
}

function reportDragging(next) {
  if (dragging === next) {
    return;
  }
  dragging = next;
  invoke("set_dragging", { dragging: next }).catch((err) => {
    console.error(err);
  });
}

function movement(event, origin) {
  return Math.hypot(event.screenX - origin.x, event.screenY - origin.y);
}

window.setPetState = setPetState;

listen("pet://state", (event) => {
  setPetState(event.payload);
});

listen("pet://joke", (event) => {
  jokeText = event.payload?.text ?? "";
  fontMaxPx = clampFontSize(event.payload?.font_size ?? FONT_DEFAULT_PX);
  applyPopupOpacity(event.payload?.popup_opacity ?? OPACITY_DEFAULT);
  chunkDelayMs = clampDelayMs(event.payload?.chunk_delay_ms ?? DELAY_DEFAULT_MS);
  if (root.classList.contains("talking")) {
    startPresentation();
  }
});

listen("pet://clear", () => {
  stopChunks();
  jokeText = "";
  speechText.textContent = "";
});

listen("pet://sound", (event) => {
  const sound = sfx[event.payload?.kind];
  if (!sound) {
    return;
  }
  sound.pause();
  sound.currentTime = 0;
  sound.loop = false;
  sound.play().catch((err) => {
    console.error(err);
  });
});

pet.addEventListener("dragstart", (event) => {
  event.preventDefault();
});

pet.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) {
    return;
  }
  event.preventDefault();
  press = { x: event.screenX, y: event.screenY, dragged: false };
});

window.addEventListener("pointermove", (event) => {
  if (!press || press.dragged) {
    return;
  }
  if (movement(event, press) <= CLICK_SLOP) {
    return;
  }
  press.dragged = true;
  reportDragging(true);
  overlay.startDragging();
});

window.addEventListener(
  "pointerup",
  (event) => {
    if (!press) {
      reportDragging(false);
      return;
    }
    const wasDrag = press.dragged;
    const origin = press;
    press = null;
    if (wasDrag) {
      reportDragging(false);
      return;
    }
    if (movement(event, origin) <= CLICK_SLOP) {
      invoke("heckle", { trigger: "click" }).catch((err) => {
        console.error(err);
      });
    }
  },
  true,
);

window.addEventListener(
  "pointercancel",
  () => {
    press = null;
    reportDragging(false);
  },
  true,
);

speech.addEventListener("pointerdown", (event) => {
  event.preventDefault();
  event.stopPropagation();
});

speech.addEventListener("click", (event) => {
  event.preventDefault();
  event.stopPropagation();
  stopChunks();
  invoke("dismiss_joke").catch((err) => {
    console.error(err);
  });
});
