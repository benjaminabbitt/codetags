// An overloaded function: two signatures, one implementation.

export function pad(value: number): string;
export function pad(value: string, width: number): string;
export function pad(value: number | string, width = 8): string {
  return String(value).padStart(width);
}

export function double(value: number): number {
  return value * 2;
}
