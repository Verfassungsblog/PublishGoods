const path = require('path');

module.exports = {
    mode: 'production',
    entry: {
        editor: { import: './Editor/Main.ts', dependOn: 'vendor' },
        settings: { import: './Settings.ts', dependOn: 'vendor' },
        import: { import: './Import.ts', dependOn: 'vendor' },
        user_tools: { import: './UserTools.ts', dependOn: 'vendor' },
        template_editor: { import: './TemplateEditor.ts', dependOn: 'vendor' },
        export: { import: './Export.ts', dependOn: 'vendor' },
        vendor: ['yjs', 'pdfjs-dist', 'handlebars', '@editorjs/editorjs'],
    },
    devtool: 'inline-source-map',
    module: {
        rules: [
            {
                test: /\.tsx?$/,
                use: 'ts-loader',
                exclude: /node_modules/,
            },
        ],
    },
    resolve: {
        extensions: ['.tsx', '.ts', '.js'],
    },
    output: {
        filename: '[name].js',
        path: path.resolve(__dirname, '../static/js'),
    },
    optimization: {
        usedExports: true,
        splitChunks: {
            cacheGroups: {
                vendor: {
                    test: /[\\/]node_modules[\\/]/,
                    name: 'vendor',
                    chunks: 'all',
                },
            },
        },
        runtimeChunk: 'single',
    },
};