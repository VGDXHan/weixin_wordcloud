import WordCloud from "wordcloud";
import type { WordFreq } from "./types";

const PALETTE = [
  "#2563eb", "#7c3aed", "#059669", "#d97706",
  "#dc2626", "#0891b2", "#db2777", "#4f46e5",
];

export function renderCloud(canvas: HTMLCanvasElement, freqs: WordFreq[]): void {
  if (freqs.length === 0) {
    const ctx = canvas.getContext("2d");
    if (ctx) ctx.clearRect(0, 0, canvas.width, canvas.height);
    return;
  }

  const max = freqs[0].count || 1;
  const list: Array<[string, number]> = freqs.map((f) => [
    f.word,
    Math.max(12, Math.round((f.count / max) * 88) + 12),
  ]);

  WordCloud(canvas, {
    list,
    gridSize: 8,
    weightFactor: (size: number) => size,
    fontFamily: '"Microsoft YaHei", "PingFang SC", sans-serif',
    color: (_word: string, weight: string | number) =>
      PALETTE[Math.floor(Number(weight)) % PALETTE.length],
    backgroundColor: "#ffffff",
    rotateRatio: 0.35,
    rotationSteps: 2,
    minSize: 12,
    drawOutOfBound: false,
    shrinkToFit: true,
  });
}

export function exportPng(canvas: HTMLCanvasElement): string {
  return canvas.toDataURL("image/png");
}
