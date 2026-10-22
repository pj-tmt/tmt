import { createElement, forwardRef } from 'react';
import * as lucide from 'lucide-react';

/**
 * The Lucide icons a page finds on `tmt.icons` (#1693). The frame has no bundler, so only the icons
 * listed here ship: the icons the Colab app itself uses plus a general set. Adding one is a single
 * reviewed line; keep the names in alphabetical order and use only static `lucide.Name` members,
 * which is what lets the build drop every icon that is not listed.
 */
const listed = {
  Archive: lucide.Archive,
  ArchiveRestore: lucide.ArchiveRestore,
  ArrowDown: lucide.ArrowDown,
  ArrowLeft: lucide.ArrowLeft,
  ArrowRight: lucide.ArrowRight,
  ArrowUp: lucide.ArrowUp,
  ArrowUpDown: lucide.ArrowUpDown,
  ArrowUpRight: lucide.ArrowUpRight,
  AtSign: lucide.AtSign,
  Ban: lucide.Ban,
  Bell: lucide.Bell,
  BellOff: lucide.BellOff,
  Bookmark: lucide.Bookmark,
  Bot: lucide.Bot,
  Calendar: lucide.Calendar,
  CalendarClock: lucide.CalendarClock,
  ChartColumn: lucide.ChartColumn,
  ChartLine: lucide.ChartLine,
  ChartPie: lucide.ChartPie,
  Check: lucide.Check,
  CheckCheck: lucide.CheckCheck,
  ChevronDown: lucide.ChevronDown,
  ChevronLeft: lucide.ChevronLeft,
  ChevronRight: lucide.ChevronRight,
  ChevronUp: lucide.ChevronUp,
  ChevronsUpDown: lucide.ChevronsUpDown,
  Circle: lucide.Circle,
  CircleAlert: lucide.CircleAlert,
  CircleCheck: lucide.CircleCheck,
  CircleDot: lucide.CircleDot,
  CircleHelp: lucide.CircleHelp,
  CircleSlash: lucide.CircleSlash,
  CircleX: lucide.CircleX,
  Clipboard: lucide.Clipboard,
  ClipboardCheck: lucide.ClipboardCheck,
  Clock: lucide.Clock,
  Code: lucide.Code,
  Columns3: lucide.Columns3,
  Copy: lucide.Copy,
  CornerDownRight: lucide.CornerDownRight,
  Database: lucide.Database,
  Diamond: lucide.Diamond,
  Download: lucide.Download,
  Ellipsis: lucide.Ellipsis,
  ExternalLink: lucide.ExternalLink,
  Eye: lucide.Eye,
  EyeOff: lucide.EyeOff,
  File: lucide.File,
  FileCode: lucide.FileCode,
  FileImage: lucide.FileImage,
  FileText: lucide.FileText,
  Flag: lucide.Flag,
  Folder: lucide.Folder,
  FolderOpen: lucide.FolderOpen,
  Funnel: lucide.Funnel,
  GitBranch: lucide.GitBranch,
  Globe: lucide.Globe,
  Hash: lucide.Hash,
  Hourglass: lucide.Hourglass,
  House: lucide.House,
  Image: lucide.Image,
  Info: lucide.Info,
  Layers: lucide.Layers,
  LayoutGrid: lucide.LayoutGrid,
  Link: lucide.Link,
  List: lucide.List,
  ListChecks: lucide.ListChecks,
  LoaderCircle: lucide.LoaderCircle,
  Lock: lucide.Lock,
  LockOpen: lucide.LockOpen,
  Mail: lucide.Mail,
  Maximize2: lucide.Maximize2,
  Menu: lucide.Menu,
  MessageCircle: lucide.MessageCircle,
  MessageSquare: lucide.MessageSquare,
  Minimize2: lucide.Minimize2,
  Minus: lucide.Minus,
  Moon: lucide.Moon,
  MousePointer2: lucide.MousePointer2,
  MoveRight: lucide.MoveRight,
  OctagonAlert: lucide.OctagonAlert,
  Paperclip: lucide.Paperclip,
  Pause: lucide.Pause,
  Pencil: lucide.Pencil,
  Pin: lucide.Pin,
  PinOff: lucide.PinOff,
  Play: lucide.Play,
  Plus: lucide.Plus,
  Redo2: lucide.Redo2,
  RefreshCw: lucide.RefreshCw,
  RotateCcw: lucide.RotateCcw,
  RotateCw: lucide.RotateCw,
  Rows3: lucide.Rows3,
  Save: lucide.Save,
  Search: lucide.Search,
  Send: lucide.Send,
  Settings: lucide.Settings,
  Share2: lucide.Share2,
  ShieldCheck: lucide.ShieldCheck,
  SlidersHorizontal: lucide.SlidersHorizontal,
  Square: lucide.Square,
  SquareKanban: lucide.SquareKanban,
  Sun: lucide.Sun,
  Table: lucide.Table,
  Tag: lucide.Tag,
  Terminal: lucide.Terminal,
  ThumbsDown: lucide.ThumbsDown,
  ThumbsUp: lucide.ThumbsUp,
  Timer: lucide.Timer,
  Trash2: lucide.Trash2,
  TriangleAlert: lucide.TriangleAlert,
  Undo2: lucide.Undo2,
  Upload: lucide.Upload,
  User: lucide.User,
  UserRound: lucide.UserRound,
  Users: lucide.Users,
  X: lucide.X,
} satisfies Record<string, lucide.LucideIcon>;

/**
 * Every icon draws square caps and miter joins in `currentColor` at stroke 1.75 and size 16, the
 * Colab look. They are defaults, not a stylesheet: a page that passes its own prop gets its own
 * value. An icon is decorative (Lucide sets `aria-hidden`) unless the page names it with
 * `aria-label` or `title`; then it is `role="img"`, and `title` becomes the SVG's `<title>`.
 */
function withPageDefaults(name: string, Icon: lucide.LucideIcon): lucide.LucideIcon {
  const Wrapped = forwardRef<SVGSVGElement, lucide.LucideProps & { title?: string }>(
    (
      { color, size, strokeWidth, strokeLinecap, strokeLinejoin, title, children, ...rest },
      ref,
    ) => {
      const named = title !== undefined || rest['aria-label'] !== undefined;
      return createElement(
        Icon,
        {
          color: color ?? 'currentColor',
          size: size ?? 16,
          strokeWidth: strokeWidth ?? 1.75,
          strokeLinecap: strokeLinecap ?? 'square',
          strokeLinejoin: strokeLinejoin ?? 'miter',
          // Only set when needed: Lucide treats any `role` key, even undefined, as a label.
          ...(named && rest.role === undefined ? { role: 'img' } : {}),
          ...rest,
          ref,
        },
        title === undefined
          ? children
          : [createElement('title', { key: 'title' }, title), children],
      );
    },
  );
  Wrapped.displayName = name;
  return Wrapped;
}

export const icons: Readonly<Record<keyof typeof listed, lucide.LucideIcon>> = Object.freeze(
  Object.fromEntries(
    Object.entries(listed).map(([name, Icon]) => [name, withPageDefaults(name, Icon)]),
  ),
) as Readonly<Record<keyof typeof listed, lucide.LucideIcon>>;
