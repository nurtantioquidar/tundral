import type { UsageSummary } from "@tundral/contracts/customer";
import { compactTokens } from "@/lib/format";

const WIDTH = 720;
const HEIGHT = 200;
const PLOT_TOP = 12;
const PLOT_BOTTOM = 176;
const PLOT_LEFT = 8;
const PLOT_RIGHT = 712;

/* Hand-built SVG stacked bars: input tokens (rule tint) under output tokens (accent). */
export function UsageChart({ summary }: { summary: UsageSummary }) {
  const days: string[] = [];
  for (let index = 0; index < summary.days; index += 1) {
    days.push(new Date(summary.since + index * 86_400_000).toISOString().slice(0, 10));
  }
  const byDay = new Map(summary.daily.map((row) => [row.day, row]));
  const max = Math.max(...days.map((dayKey) => {
    const row = byDay.get(dayKey);
    return row ? row.input_tokens + row.output_tokens : 0;
  }), 1);

  const plotHeight = PLOT_BOTTOM - PLOT_TOP;
  const step = (PLOT_RIGHT - PLOT_LEFT) / days.length;
  const barWidth = Math.max(step * 0.6, 3);

  return (
    <figure style={{ margin: 0 }}>
      <svg className="chart" viewBox={`0 0 ${WIDTH} ${HEIGHT}`} role="img" aria-label={`Tokens per day over the last ${summary.days} days`}>
        <line className="chart__grid" x1={PLOT_LEFT} y1={PLOT_BOTTOM} x2={PLOT_RIGHT} y2={PLOT_BOTTOM} />
        <line className="chart__grid" x1={PLOT_LEFT} y1={PLOT_TOP} x2={PLOT_RIGHT} y2={PLOT_TOP} strokeDasharray="2 4" />
        <text className="chart__axis" x={PLOT_LEFT} y={PLOT_TOP - 3}>
          {compactTokens(max)} tokens
        </text>
        {days.map((dayKey, index) => {
          const row = byDay.get(dayKey);
          const x = PLOT_LEFT + index * step + (step - barWidth) / 2;
          if (!row) return null;
          const inputHeight = (row.input_tokens / max) * plotHeight;
          const outputHeight = (row.output_tokens / max) * plotHeight;
          return (
            <g key={dayKey}>
              <title>{`${dayKey} — in ${compactTokens(row.input_tokens)} · out ${compactTokens(row.output_tokens)} · ${row.requests} req`}</title>
              <rect
                className="chart__bar--cached"
                x={x}
                y={PLOT_BOTTOM - inputHeight}
                width={barWidth}
                height={inputHeight}
                fill="var(--color-rule-2)"
              />
              <rect
                className="chart__bar"
                x={x}
                y={PLOT_BOTTOM - inputHeight - outputHeight}
                width={barWidth}
                height={outputHeight}
              />
            </g>
          );
        })}
        <text className="chart__axis" x={PLOT_LEFT} y={HEIGHT - 8}>
          {days[0]}
        </text>
        <text className="chart__axis" x={PLOT_RIGHT} y={HEIGHT - 8} textAnchor="end">
          {days[days.length - 1]}
        </text>
      </svg>
      <figcaption className="legend">
        <span>
          <span className="legend__swatch" style={{ background: "var(--color-rule-2)" }} />
          input tokens
        </span>
        <span>
          <span className="legend__swatch" style={{ background: "var(--color-accent)" }} />
          output tokens
        </span>
      </figcaption>
    </figure>
  );
}
