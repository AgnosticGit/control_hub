/** Pixel positions inside the expanded service card. The header stays clear of metric rows. */
export const SERVICE_CARD_LAYOUT = {
  icon: { x: 12, y: 7, width: 30, height: 30 },
  title: { x: 52, y: 10, width: 110 },
  cpu: { label: { x: 12, y: 39 }, value: { x: 142, y: 39 }, barY: 42 },
  memory: { label: { x: 12, y: 56 }, value: { x: 142, y: 56 }, barY: 59 },
  network: { receive: { x: 12, y: 74 }, transmit: { x: 112, y: 74 } },
  requests: { incoming: { x: 12, y: 88 }, outgoing: { x: 112, y: 88 } },
  textHeight: 13,
} as const;
