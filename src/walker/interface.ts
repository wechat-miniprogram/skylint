type WalkerReturnType = boolean | void;

export interface DefaultWalkerContext<T> {
    parent: T;
}

export interface WalkerCallback<T, K = T> {
  (node: T): WalkerReturnType;
  /**
   * @arg ctx {for top-level node, e.g. Document, the parent node is itself}
   */
  (node: T, ctx: K): WalkerReturnType;
}

export type Walker<T, K = null> = K extends Object
  ? (node: T, callback: (node: T, ctx: K) => WalkerReturnType, ctx: K) => WalkerReturnType
  : (node: T, callback: (node: T) => WalkerReturnType) => WalkerReturnType;
