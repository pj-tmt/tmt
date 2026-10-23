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
import { PageSectionsSection, TablesSection, VisuallyHiddenSection } from './layout.js';
import { NotificationsSection, OverlaysSection, ShellSection } from './frames.js';
import { ChartsSection } from './charts.js';
import { UiLayerSection } from './ui-layer.js';
import { UiOverlaysSection } from './ui-overlays.js';
import {
  AvatarSection,
  ChoiceSection,
  CountChipSection,
  PagerSection,
  StatusTileSection,
  CardSection,
} from './pieces.js';

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
      <CountChipSection />
      <AvatarSection />
      <ChoiceSection />
      <PagerSection />
      <StatusTileSection />
      <CardSection />
      <MenusSection />
      <TabsSection />
      <ListSection />
      <PageSectionsSection />
      <TablesSection />
      <VisuallyHiddenSection />
      <CommandSection />
      <NoticeSection />
      <OverlaysSection />
      <UiOverlaysSection />
      <NotificationsSection />
      <ChartsSection />
      <UiLayerSection />
    </>
  );
}
