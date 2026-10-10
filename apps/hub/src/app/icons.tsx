/* Pixel art as crisp inline SVG. The artwork and the rasteriser live in pixel-art.ts. */

import { type ArtSpec, ICONS, LOGOS, rasterise } from "./pixel-art";

const ART: Record<string, ArtSpec> = { ...ICONS, ...LOGOS };

export function PixelIcon({
  id,
  label,
  className,
}: {
  id: keyof typeof ICONS | keyof typeof LOGOS;
  label?: string;
  className?: string;
}) {
  const { size, rects } = rasterise(ART[id]);
  return (
    <svg
      className={className}
      viewBox={`0 0 ${size} ${size}`}
      shapeRendering="crispEdges"
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      {rects.map((r) => (
        <rect key={`${r.x}-${r.y}`} x={r.x} y={r.y} width={r.w} height={1} className={r.cls} />
      ))}
    </svg>
  );
}
