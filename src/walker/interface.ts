export type Walker<T> = (node: T, callback: (node: T) => boolean | void) => boolean | void;
