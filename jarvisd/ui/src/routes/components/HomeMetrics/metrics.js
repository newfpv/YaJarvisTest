export function readMetrics(system = {}, points) {
  const { temp_c, mem_total_mb, mem_avail_mb, wifi_dbm } = system;
  const used = (mem_total_mb || 0) - (mem_avail_mb || 0);
  const last = points.at(-1) || {};
  const cpu = points.length > 1 ? Math.round(last.cpu) : null;
  const jarvisCpu = points.length > 1 ? Math.round(last.me || 0) : null;
  const wifi = wifi_dbm || null;
  const signal = wifi == null ? "" : wifi >= -60 ? "сигнал отличный" : wifi >= -70 ? "сигнал нормальный" : "сигнал слабый — ответы могут задерживаться";
  return [
    {
      id: "temp", label: "Температура", value: temp_c ? Math.round(temp_c) : null, unit: "°C",
      fraction: (temp_c || 0) / 85, description: "процессор", series: points.map((p) => p.temp || null),
      level: temp_c > 75 ? "bad" : temp_c > 65 ? "warn" : "",
    },
    {
      id: "mem", label: "Память", value: mem_total_mb ? used : null, unit: `из ${mem_total_mb} МБ`,
      fraction: used / (mem_total_mb || 1), description: mem_total_mb ? `свободно ${mem_avail_mb} МБ` : "",
      series: points.map((p) => p.mem),
      tip: "Сколько оперативной памяти колонки занято. Джарвис занимает около 25 МБ.",
    },
    {
      id: "cpu", label: "Процессор", value: cpu, unit: "%",
      fraction: (cpu || 0) / 100, description: jarvisCpu != null ? `из них Джарвис — ${jarvisCpu}%` : "все ядра колонки",
      series: points.slice(1).map((p) => p.cpu), level: cpu > 85 ? "warn" : "",
    },
    {
      id: "wifi", label: "Wi-Fi", value: wifi, unit: "дБм", description: signal,
      fraction: wifi == null ? 0 : (wifi + 90) / 60, series: points.map((p) => p.wifi || null),
      level: wifi < -70 ? "warn" : "",
      tip: "Уровень сигнала: от −30 до −60 дБм — отлично, до −70 — нормально, ниже — связь может рваться, и Джарвис будет отвечать с задержкой.",
    },
  ];
}

export function sparkPaths(series, width, height) {
  const values = series.filter((value) => value != null && !isNaN(value));
  if (values.length < 2) return null;
  let low = Math.min(...values);
  let high = Math.max(...values);
  if (high - low < 1e-6) {
    low -= 1;
    high += 1;
  }
  const top = height * 0.3;
  const line = values.map((value, index) => {
    const x = index / (values.length - 1) * width;
    const y = top + (1 - (value - low) / (high - low)) * (height - top - 2);
    return `${index ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`;
  }).join("");
  return { line, area: `${line}L${width} ${height}L0 ${height}Z` };
}
