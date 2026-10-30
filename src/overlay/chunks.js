const MAX_CHUNKS = 64;

export function splitToViewport(text, fits, maxChunks = MAX_CHUNKS) {
  const source = String(text ?? "").trim();
  if (!source) {
    return [""];
  }
  const greedy = packGreedy(source, fits, maxChunks);
  if (greedy.length <= 1) {
    return greedy;
  }
  return packEven(source, greedy.length, fits) ?? greedy;
}

function packGreedy(text, fits, maxChunks) {
  const chunks = [];
  let rest = text;
  let guard = 0;
  while (rest.length && guard < maxChunks) {
    guard += 1;
    if (fits(rest)) {
      chunks.push(rest);
      rest = "";
      break;
    }
    const n = greedyBreak(rest, longestPrefix(rest, fits));
    const chunk = rest.slice(0, n).trim();
    if (!chunk) {
      rest = rest.slice(Math.max(n, 1)).trim();
      continue;
    }
    chunks.push(chunk);
    rest = rest.slice(n).trim();
  }
  if (rest.length) {
    chunks.push(rest);
  }
  return chunks.length ? chunks : [text];
}

function packEven(text, pages, fits) {
  const chunks = [];
  let rest = text;
  for (let left = pages; left > 1; left -= 1) {
    if (fits(rest)) {
      chunks.push(rest);
      return chunks;
    }
    const maxLen = longestPrefix(rest, fits);
    if (maxLen <= 0) {
      return null;
    }
    const minLen = rest.length - maxLen * (left - 1);
    const target = Math.round(rest.length / left);
    const n = evenBreak(rest, target, maxLen, minLen);
    const chunk = rest.slice(0, n).trim();
    if (!chunk) {
      return null;
    }
    chunks.push(chunk);
    rest = rest.slice(n).trim();
    if (!rest) {
      return chunks;
    }
  }
  if (!fits(rest)) {
    return null;
  }
  chunks.push(rest);
  return chunks;
}

function longestPrefix(text, fits) {
  if (fits(text)) {
    return text.length;
  }
  let lo = 0;
  let hi = text.length;
  let best = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (fits(text.slice(0, mid))) {
      best = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return best;
}

function greedyBreak(text, maxLen) {
  if (maxLen >= text.length) {
    return text.length;
  }
  if (maxLen <= 0) {
    return Math.min(1, text.length);
  }
  const prefix = text.slice(0, maxLen);
  const ws = Math.max(
    prefix.lastIndexOf(" "),
    prefix.lastIndexOf("\n"),
    prefix.lastIndexOf("\t"),
  );
  if (ws > 0) {
    return ws;
  }
  return maxLen;
}

function evenBreak(text, target, maxLen, minLen) {
  if (maxLen >= text.length) {
    return text.length;
  }
  if (maxLen <= 0) {
    return Math.min(1, text.length);
  }
  const lo = Math.max(1, Math.min(minLen, maxLen));
  const hi = maxLen;
  const aim = Math.max(lo, Math.min(target, hi));
  const bonus = Math.max(4, Math.floor((hi - lo) * 0.25));
  let best = 0;
  let bestScore = Infinity;
  for (let i = lo; i < hi; i += 1) {
    if (!isSoftBreak(text, i)) {
      continue;
    }
    let score = Math.abs(i - aim);
    if (isSentenceBreak(text, i)) {
      score -= bonus;
    }
    if (score < bestScore) {
      bestScore = score;
      best = i;
    }
  }
  if (bestScore === Infinity) {
    return aim;
  }
  return best;
}

function isSoftBreak(text, i) {
  return /\s/.test(text[i]);
}

function isSentenceBreak(text, i) {
  return /[.!?]["')\]]*$/.test(text.slice(0, i).trimEnd());
}
