export function createSpecIsolation({ drain, clear }) {
  let lastFile = null;
  return (file) => {
    if (file === lastFile) return;
    drain();
    clear();
    lastFile = file;
  };
}
