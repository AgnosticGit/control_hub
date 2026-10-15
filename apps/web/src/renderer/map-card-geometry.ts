export interface CardSize {
  width: number;
  height: number;
}

export interface RouteBox {
  hw: number;
  hh: number;
  portSpan: number;
}

export const COMPACT_CARD_SIZE: CardSize = { width: 172, height: 56 };
export const EXPANDED_CARD_SIZE: CardSize = { width: 220, height: 104 };

/** Shrink metric cards to the lowest visible row; the header remains the same size. */
export function serviceMetricCardSize(rows: readonly string[]): CardSize {
  if (rows.length === 0) return { width: EXPANDED_CARD_SIZE.width, height: COMPACT_CARD_SIZE.height };
  const lastRowY = rows.reduce((latest, row) => {
    const y = row === "cpu" ? 39 : row === "memory" ? 56 : row === "network" ? 74 : row === "requests" ? 88 : 0;
    return Math.max(latest, y);
  }, 0);
  return { width: EXPANDED_CARD_SIZE.width, height: Math.max(COMPACT_CARD_SIZE.height, Math.min(EXPANDED_CARD_SIZE.height, lastRowY + 16)) };
}

export function cardGeometry(expanded: boolean): CardSize {
  return expanded ? EXPANDED_CARD_SIZE : COMPACT_CARD_SIZE;
}

export function routeBoxForCard(size: CardSize): RouteBox {
  return { hw: size.width / 2 + 4, hh: size.height / 2 + 4, portSpan: size.height - 12 };
}
