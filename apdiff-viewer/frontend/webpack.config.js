const path = require('path');
const MiniCssExtractPlugin = require('mini-css-extract-plugin');

module.exports = (_, { mode }) => ({
    context: __dirname,
    entry: path.resolve(__dirname, 'src', 'index.tsx'),
    target: 'web',
    devtool: mode === 'production' ? false : 'inline-source-map',
    output: {
        path: path.resolve(__dirname, 'build'),
        clean: true,
        publicPath: '/dist/',
        filename: 'bundle.js',
        globalObject: 'this',
    },
    module: {
        rules: [
            {
                test: /\.tsx?$/,
                use: 'ts-loader',
                exclude: /node_modules/,
            },
            {
              test: /\.css$/,
              use: [
                {
                  loader: MiniCssExtractPlugin.loader,
                  options: {
                    esModule: true,
                  },
                },
                {
                  loader: 'css-loader',
                  options: {
                    importLoaders: 0,
                  },
                },
              ],
            },
        ]
    },
    plugins: [
      new MiniCssExtractPlugin({
        filename: 'style.css',
        ignoreOrder: false,
      }),
    ],
    resolve: {
        extensions: ['.tsx', '.ts', '.js', '.css'],
    },
});
