<script setup>
import { VPTeamMembers } from 'vitepress/theme'

const members = [
  {
    avatar: 'https://www.github.com/aperio-search.png',
    name: 'Aperio',
    title: 'Search Engine',
    links: [
      { icon: 'github', link: 'https://github.com/aperio-search/aperio' },
      { icon: 'x', link: 'https://x.com/aperiosearch' },
    ]
  },
  {
    avatar: 'https://www.github.com/andresribeiro.png',
    name: 'André Ribeiro',
    title: 'Creator',
    links: [
      { icon: 'github', link: 'https://github.com/andresribeiro' },
      { icon: 'x', link: 'https://x.com/andresribeiroo' },
      { icon: 'instagram', link: 'https://instagram.com/andresribeiroo' }
    ]
  },
]
</script>

# About

Love using Aperio? Consider becoming a sponsor! As an independent open-source project, we rely on community backing to keep Aperio screamingly fast, ultra-lean, and actively maintained. Take a look at our [GitHub Sponsors](https://github.com/sponsors/andresribeiro) page to see how you can help.

Thank you for using Aperio!

<VPTeamMembers size="small" :members />
