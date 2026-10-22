import { TokensSection } from './tokens.js';
import {
  ActionsSection,
  BrandSection,
  BusySection,
  ChipsSection,
  CommandSection,
  FieldsSection,
  HeaderSection,
  IconActionsSection,
  ListSection,
  MenusSection,
  NoticeSection,
  TabsSection,
  ToggleSection,
  CheckboxSection,
} from './components.js';
import { NotificationsSection, OverlaysSection, ShellSection } from './frames.js';
import { UiLayerSection } from './ui-layer.js';

/** Every gallery section, in the order of `sections` (covered.ts). */
export function GallerySections() {
  return (
    <>
      <TokensSection />
      <BrandSection />
      <ShellSection />
      <HeaderSection />
      <ActionsSection />
      <IconActionsSection />
      <BusySection />
      <FieldsSection />
      <ToggleSection />
      <CheckboxSection />
      <ChipsSection />
      <MenusSection />
      <TabsSection />
      <ListSection />
      <CommandSection />
      <NoticeSection />
      <OverlaysSection />
      <NotificationsSection />
      <UiLayerSection />
    </>
  );
}
