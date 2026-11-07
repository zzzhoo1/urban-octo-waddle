import { PieChart, Users, Key, Cpu, BarChart, List, Clock, Settings, Menu as MenuIcon, LogOut as LogOutIcon } from "lucide-react";

export const Icons = {
  PieChart,
  Users,
  Key,
  Cpu,
  BarChart,
  List,
  Clock,
  Settings,
  Menu: MenuIcon,
  LogOut: LogOutIcon,
};

export type IconName = keyof typeof Icons;
