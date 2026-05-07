// Generic Mantine table with the loading / empty / error
// states every list view needs. Columns are declarative —
// each column has a label, a key into the row, and an
// optional render function. Click-through is opt-in via
// `onRowClick`; the row is rendered with a pointer cursor
// when set.

import { type ReactNode } from 'react';
import { Center, Loader, Stack, Table, Text } from '@mantine/core';

export interface ColumnDef<T> {
  key: keyof T & string;
  label: string;
  render?: (value: T[keyof T & string], row: T) => ReactNode;
}

interface Props<T> {
  title?: string;
  description?: string;
  loading?: boolean;
  error?: unknown;
  items: T[];
  columns: ColumnDef<T>[];
  rowKey: (row: T) => string | number;
  onRowClick?: (row: T) => void;
  emptyMessage?: string;
}

export function DataTable<T>({
  title,
  description,
  loading,
  error,
  items,
  columns,
  rowKey,
  onRowClick,
  emptyMessage = 'No items.',
}: Props<T>) {
  return (
    <Stack gap="md">
      {(title || description) && (
        <Stack gap={4}>
          {title && <Text fw={600}>{title}</Text>}
          {description && (
            <Text size="sm" c="dimmed">
              {description}
            </Text>
          )}
        </Stack>
      )}

      {loading ? (
        <Center mih={120}>
          <Loader size="sm" />
        </Center>
      ) : error ? (
        <Text c="red" size="sm">
          Failed to load. See the error toast for details.
        </Text>
      ) : items.length === 0 ? (
        <Text c="dimmed" size="sm">
          {emptyMessage}
        </Text>
      ) : (
        <Table highlightOnHover striped withTableBorder>
          <Table.Thead>
            <Table.Tr>
              {columns.map((col) => (
                <Table.Th key={col.key}>{col.label}</Table.Th>
              ))}
            </Table.Tr>
          </Table.Thead>
          <Table.Tbody>
            {items.map((row) => (
              <Table.Tr
                key={rowKey(row)}
                onClick={onRowClick ? () => onRowClick(row) : undefined}
                style={onRowClick ? { cursor: 'pointer' } : undefined}
              >
                {columns.map((col) => {
                  const v = row[col.key];
                  return (
                    <Table.Td key={col.key}>
                      {col.render ? col.render(v, row) : renderDefault(v)}
                    </Table.Td>
                  );
                })}
              </Table.Tr>
            ))}
          </Table.Tbody>
        </Table>
      )}
    </Stack>
  );
}

function renderDefault(v: unknown): ReactNode {
  if (v === null || v === undefined) return '—';
  if (typeof v === 'boolean') return v ? 'true' : 'false';
  return String(v);
}
