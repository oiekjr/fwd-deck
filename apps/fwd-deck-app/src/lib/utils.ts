import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Tailwind CSS のクラス名を安全に結合する
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
