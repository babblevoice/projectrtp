module.exports = {
  "env": {
    "browser": false,
    "node": true,
    "mocha": true,
    "commonjs": true,
    "es2021": true
  },
  "extends": "eslint:recommended",
  "overrides": [
  ],
  "parserOptions": {
    "ecmaVersion": "latest"
  },
  "ignorePatterns": [ "out" ],
  "rules": {
    "require-atomic-updates": [
      "error"
    ],
    "no-invalid-this": [
      "error"
    ],
    "no-useless-call": [
      "error"
    ],
    "no-useless-return": [
      "error"
    ],
    "no-var": [
      "error"
    ],
    "prefer-const": [
      "error"
    ],
    "complexity": [
      "error",
      10
    ],
    "max-depth": [
      "error",
      5
    ],
    "no-eval": [
      "error"
    ],
    "indent": [
      "error",
      2
    ],
    "linebreak-style": [
      "error",
      "unix"
    ],
    "quotes": [
      "error",
      "double"
    ],
    "semi": [
      "error",
      "never"
    ],
    "yoda": [
      "error",
      "always"
    ]
  }
}
