// Project-workspace shell. Left rail with the six sections
// from `UI.md` "Information architecture / Rail layout":
// Demand, Inventory, Config, Reports, Library, Settings.
// Most sections render placeholder rows until 7.6 lands the
// real audit views.
//
// Used by every `/orgs/{org_id}/projects/{project_id}/*`
// route via TanStack Router's nested layout pattern.

import type { ReactNode } from 'react';
import { AppShell, Group, NavLink, Stack, Text, Title } from '@mantine/core';
import { Link, useLocation } from '@tanstack/react-router';

interface RailItem {
  to: string;
  label: string;
}

interface RailSection {
  title: string;
  items: RailItem[];
}

interface Props {
  orgId: string;
  projectId: string;
  projectName?: string;
  children: ReactNode;
}

function buildSections(orgId: string, projectId: string): RailSection[] {
  const projectBase = `/orgs/${orgId}/projects/${projectId}`;
  return [
    {
      title: 'Demand',
      items: [
        { to: `${projectBase}/advertisers`, label: 'Advertisers' },
        { to: `${projectBase}/campaigns`, label: 'Campaigns' },
        { to: `${projectBase}/flights`, label: 'Flights' },
        { to: `${projectBase}/ads`, label: 'Ads' },
        { to: `${projectBase}/creatives`, label: 'Creatives' },
      ],
    },
    {
      title: 'Inventory',
      items: [
        { to: `${projectBase}/sites`, label: 'Sites' },
        { to: `${projectBase}/zones`, label: 'Zones' },
      ],
    },
    {
      title: 'Config',
      items: [
        { to: `${projectBase}/templates`, label: 'Creative templates' },
        { to: `${projectBase}/taxonomy`, label: 'Taxonomy' },
      ],
    },
    {
      title: 'Reports',
      items: [
        { to: `${projectBase}/reports`, label: 'Rollups' },
        { to: `${projectBase}/reports/test`, label: 'Decision tester' },
        { to: `${projectBase}/reports/explain`, label: 'Decision explainer' },
        { to: `${projectBase}/reports/events`, label: 'Events tail' },
      ],
    },
    {
      title: 'Library',
      items: [{ to: `/orgs/${orgId}/library`, label: 'Ad library' }],
    },
    {
      title: 'Settings',
      items: [
        { to: `/orgs/${orgId}/members`, label: 'Members' },
        { to: `/orgs/${orgId}/tokens`, label: 'Tokens' },
      ],
    },
  ];
}

export function WorkspaceShell({ orgId, projectId, projectName, children }: Props) {
  const location = useLocation();
  const sections = buildSections(orgId, projectId);
  const projectBase = `/orgs/${orgId}/projects/${projectId}`;

  return (
    <AppShell header={{ height: 56 }} navbar={{ width: 240, breakpoint: 'sm' }} padding="md">
      <AppShell.Header>
        <Group h="100%" px="md" justify="space-between">
          <Group>
            <Link to="/" style={{ textDecoration: 'none', color: 'inherit' }}>
              <Title order={4}>Knievel Admin</Title>
            </Link>
            {projectName && (
              <>
                <Text c="dimmed">/</Text>
                <Link to={projectBase} style={{ textDecoration: 'none', color: 'inherit' }}>
                  <Text fw={500}>{projectName}</Text>
                </Link>
              </>
            )}
          </Group>
          <Link to="/oidc/logout" style={{ color: 'inherit' }}>
            <Text size="sm">Sign out</Text>
          </Link>
        </Group>
      </AppShell.Header>

      <AppShell.Navbar p="xs">
        <Stack gap="md">
          {sections.map((section) => (
            <Stack key={section.title} gap={2}>
              <Text size="xs" tt="uppercase" c="dimmed" fw={700} pl="sm">
                {section.title}
              </Text>
              {section.items.map((item) => (
                <NavLink
                  key={item.to}
                  component={Link}
                  to={item.to}
                  label={item.label}
                  active={location.pathname.startsWith(item.to)}
                />
              ))}
            </Stack>
          ))}
        </Stack>
      </AppShell.Navbar>

      <AppShell.Main>{children}</AppShell.Main>
    </AppShell>
  );
}
