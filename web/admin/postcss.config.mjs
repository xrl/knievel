// Mantine v7 ships its CSS via PostCSS — `postcss-preset-mantine`
// resolves Mantine's CSS-vars helpers and `postcss-simple-vars`
// supports the breakpoint variables. See
// https://mantine.dev/styles/postcss-preset/.
export default {
  plugins: {
    'postcss-preset-mantine': {},
    'postcss-simple-vars': {
      variables: {
        'mantine-breakpoint-xs': '36em',
        'mantine-breakpoint-sm': '48em',
        'mantine-breakpoint-md': '62em',
        'mantine-breakpoint-lg': '75em',
        'mantine-breakpoint-xl': '88em',
      },
    },
  },
};
