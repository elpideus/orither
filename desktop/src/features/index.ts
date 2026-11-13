import type { Feature } from "../core/feature";
import { apps } from "./apps";
import { calls } from "./calls";
import { clipboard } from "./clipboard";
import { files } from "./files";
import { gallery } from "./gallery";
import { helper } from "./helper";
import { input } from "./input";
import { messages } from "./messages";
import { settings } from "./settings";
import { transfers } from "./transfers";
import { updatesFeature } from "./updates";
import { whatsapp } from "./whatsapp";

/** Every feature. Those with tabs appear in this order, after Home, which is ordered by use. */
export const features: Feature[] = [whatsapp, calls, clipboard, gallery, messages, files, apps, settings, transfers, helper, input, updatesFeature];
