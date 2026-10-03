// Office and Colab owners decide these new React diagnostics in
// https://github.com/pj-tmt/tmt/issues/1405. Other bundled defaults stay enabled.
export const lintConfig = {
  rules: {
    'react/set-state-in-effect': 'off',
    'react/refs': 'off',
    'react/immutability': 'off',
  },
};
