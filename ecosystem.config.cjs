module.exports = {
  apps: [
    {
      name: 'photo-gallery',
      script: './server/entry.mjs',
      env: {
        PORT: 4321,
        HOST: '127.0.0.1',
        NODE_ENV: 'production'
      }
    }
  ]
};
