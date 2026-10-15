export function createSpecIsolation(actions: { drain(): void; clear(): void }): (file: string) => void;
