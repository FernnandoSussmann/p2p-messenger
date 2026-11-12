export function shortNodeId(id: string): string {
  return id.length <= 20 ? id : `${id.slice(0, 10)}…${id.slice(-6)}`;
}
