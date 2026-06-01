const js = require( "@eslint/js" )
const globals = require( "globals" )

module.exports = [
  {
    ignores: [ "out/**" ]
  },
  js.configs.recommended,
  {
    languageOptions: {
      ecmaVersion: "latest",
      sourceType: "commonjs",
      globals: {
        ...globals.node,
        ...globals.mocha,
        ...globals.es2021
      }
    },
    rules: {
      "require-atomic-updates": [ "error" ],
      "no-invalid-this": [ "error" ],
      "no-useless-call": [ "error" ],
      "no-useless-return": [ "error" ],
      "no-var": [ "error" ],
      "prefer-const": [ "error" ],
      "complexity": [ "error", 10 ],
      "max-depth": [ "error", 5 ],
      "no-eval": [ "error" ],
      "indent": [ "error", 2 ],
      "linebreak-style": [ "error", "unix" ],
      "quotes": [ "error", "double" ],
      "semi": [ "error", "never" ],
      "yoda": [ "error", "always" ]
    }
  }
]
