"""Print candidate audio impulses and visual transitions for a captured clip."""

from __future__ import annotations

import argparse
from pathlib import Path

import av
import cv2
import numpy as np


def separated_peaks(samples: list[tuple[float, float]], distance: float, limit: int) -> list[tuple[float, float]]:
    selected: list[tuple[float, float]] = []
    for timestamp, value in sorted(samples, key=lambda item: item[1], reverse=True):
        if all(abs(timestamp - other) >= distance for other, _ in selected):
            selected.append((timestamp, value))
            if len(selected) == limit:
                break
    return sorted(selected)


def analyze(path: Path, contact_sheet: Path | None) -> None:
    container = av.open(path)
    audio: list[tuple[float, float]] = []
    for frame in container.decode(audio=0):
        values = frame.to_ndarray().astype(np.float32)
        rms = float(np.sqrt(np.mean(np.square(values))))
        audio.append((float(frame.time or 0.0), rms))
    container.close()

    container = av.open(path)
    visual: list[tuple[float, float]] = []
    previous: np.ndarray | None = None
    thumbnails: list[np.ndarray] = []
    for frame in container.decode(video=0):
        image = frame.to_ndarray(format="gray")
        image = cv2.resize(image, (320, 180), interpolation=cv2.INTER_AREA)
        if previous is not None:
            difference = float(np.mean(cv2.absdiff(image, previous)))
            visual.append((float(frame.time or 0.0), difference))
        previous = image
        if contact_sheet is not None and frame.time is not None and round(float(frame.time) * 5) % 2 == 0:
            color = frame.to_ndarray(format="bgr24")
            color = cv2.resize(color, (320, 180), interpolation=cv2.INTER_AREA)
            cv2.putText(color, f"{float(frame.time):.2f}s", (8, 22), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (255, 255, 255), 2)
            thumbnails.append(color)
    container.close()

    print(f"path={path}")
    print("audio_impulses=", separated_peaks(audio, 0.12, 20))
    print("visual_transitions=", separated_peaks(visual, 0.12, 20))
    if contact_sheet is not None and thumbnails:
        rows = [np.hstack(thumbnails[index:index + 5]) for index in range(0, len(thumbnails), 5)]
        width = max(row.shape[1] for row in rows)
        rows = [cv2.copyMakeBorder(row, 0, 0, 0, width - row.shape[1], cv2.BORDER_CONSTANT) for row in rows]
        cv2.imwrite(str(contact_sheet), np.vstack(rows))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("path", type=Path)
    parser.add_argument("--contact-sheet", type=Path)
    args = parser.parse_args()
    analyze(args.path, args.contact_sheet)
