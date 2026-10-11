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

export function cardGeometry(expanded: boolean): CardSize {
  return expanded ? EXPANDED_CARD_SIZE : COMPACT_CARD_SIZE;
}

export function routeBoxForCard(size: CardSize): RouteBox {
  return { hw: size.width / 2 + 4, hh: size.height / 2 + 4, portSpan: size.height - 12 };
}
