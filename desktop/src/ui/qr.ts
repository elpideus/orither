import { encode } from "uqr";

/** The modules of a QR code. The margin is left to whatever draws them. */
export const qrModules = (text: string) => encode(text, { ecc: "M", border: 0 });
