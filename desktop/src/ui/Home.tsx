import { AppDock } from "../features/apps/Dock";
import { CallCards } from "../features/calls/CallCards";
import { NotificationList } from "../features/notifications/List";
import { WhatsAppPane } from "../features/whatsapp/Pane";
import "./Home.css";

/**
 * Home, in the order the person uses Orither: their apps first, as the largest
 * targets, then WhatsApp, then calls and notifications from every phone.
 */
export function Home() {
  return (
    <div className="home">
      <AppDock />
      <div className="home-panes">
        <WhatsAppPane />
        <section className="pane home-activity" aria-label="Calls and notifications">
          <CallCards />
          <NotificationList />
        </section>
      </div>
    </div>
  );
}
