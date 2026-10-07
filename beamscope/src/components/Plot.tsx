import { useEffect, useRef } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";

interface Props {
  /** Options without width/height; the plot fills its container. */
  options: Omit<uPlot.Options, "width" | "height">;
  data: uPlot.AlignedData;
  /** Change this to rebuild the chart (e.g. when the series list changes). */
  optionsKey?: string;
  className?: string;
}

export function Plot({ options, data, optionsKey, className }: Props) {
  const el = useRef<HTMLDivElement>(null);
  const plot = useRef<uPlot | null>(null);
  const dataRef = useRef(data);
  dataRef.current = data;

  useEffect(() => {
    const div = el.current!;
    const u = new uPlot({ ...options, width: div.clientWidth || 300, height: div.clientHeight || 150 } as uPlot.Options, dataRef.current, div);
    plot.current = u;
    const ro = new ResizeObserver(() => {
      if (div.clientWidth > 0 && div.clientHeight > 0) u.setSize({ width: div.clientWidth, height: div.clientHeight });
    });
    ro.observe(div);
    return () => {
      ro.disconnect();
      u.destroy();
      plot.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [optionsKey]);

  useEffect(() => {
    plot.current?.setData(data);
  }, [data]);

  return <div ref={el} className={`plot ${className ?? ""}`} />;
}

const AXIS = {
  stroke: "#8b95a7",
  grid: { stroke: "rgba(139,149,167,0.12)", width: 1 },
  ticks: { stroke: "rgba(139,149,167,0.25)", width: 1 },
  font: "11px Inter, system-ui, sans-serif",
};

export const axisStyle = AXIS;
