import { createContext, useContext } from "react";

export interface Nav {
  /** The page shown: "home" or a feature's id. */
  page: string;
  go: (page: string) => void;
}

export const NavContext = createContext<Nav>({ page: "home", go: () => {} });

/** Which page is open, and a way to open another. */
export const useNav = () => useContext(NavContext);
