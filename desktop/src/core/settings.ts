import { useCallback, useState } from "react";

function read<T>(key: string, fallback: T): T {
  try {
    const stored = localStorage.getItem(key);
    return stored === null ? fallback : (JSON.parse(stored) as T);
  } catch {
    return fallback;
  }
}

/** A choice that belongs to this computer and is remembered between runs. */
export function useSetting<T>(name: string, fallback: T): [T, (value: T) => void] {
  const key = `orither.${name}`;
  const [value, setValue] = useState<T>(() => read(key, fallback));
  const set = useCallback(
    (next: T) => {
      try {
        localStorage.setItem(key, JSON.stringify(next));
      } catch {
        // Not remembered, but still in effect for now.
      }
      setValue(next);
    },
    [key],
  );
  return [value, set];
}

/** Reads a remembered choice outside React. */
export function setting<T>(name: string, fallback: T): T {
  return read(`orither.${name}`, fallback);
}

export function remember<T>(name: string, value: T) {
  try {
    localStorage.setItem(`orither.${name}`, JSON.stringify(value));
  } catch {
    // Not remembered.
  }
}
